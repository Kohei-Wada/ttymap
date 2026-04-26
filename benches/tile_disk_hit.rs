//! Tile fetch latency: how fast does a disk-resident tile move from
//! "user requested it" to "available for render"?
//!
//! Four benchmarks isolate where time goes:
//!
//! 1. `disk_read_only` — just read the bytes off disk (warm page
//!    cache).
//! 2. `disk_read_then_decode` — read + `decode::decode`. The
//!    synchronous cost of bringing one tile from disk to a
//!    `DecodedTile`.
//! 3. `decoder_pipeline` — end-to-end through the decoder lane:
//!    send bytes via the same `bytes_tx` the cache uses, receive
//!    `DecodedTile` on `decoded_rx`. Architectural cost of
//!    `channel-send + decoder thread + channel-recv` on top of bare
//!    `decode::decode`.
//! 4. `memory_hit` — `TileCache::get_tile` for an LRU-resident
//!    tile.
//!
//! The render-thread poll cycle is **not** measured here. Empirical
//! results (~430 µs pipeline vs. 16 ms POLL_MS) show the poll wait
//! dominates user-perceived latency; the proper fix is push-notify
//! on tile arrival (issue #62).

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use criterion::{Criterion, criterion_group, criterion_main};

use ttymap::map::tile::cache::{DiskFastPath, TileCache};
use ttymap::map::tile::decode;
use ttymap::map::tile::decoder;
use ttymap::map::tile::disk;
use ttymap::map::tile::fetch::queue::PriorityFn;
use ttymap::map::tile::fetch::{TileFetchLane, TilePriority};
use ttymap::map::tile::key::TileKey;

/// Same fixture used by `decode_tile` / `render_frame` benches —
/// real z14 mapscii.me tile, ~5 KB gzipped, ~11 KB raw, ~1000
/// features across ~15 layers.
const SAMPLE: &[u8] = include_bytes!("fixtures/z14.pbf");

struct BenchDir(PathBuf);

impl BenchDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("ttymap-bench-{}-{}", label, nanos));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for BenchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `TileFetchLane` that does nothing — the bench drives the disk
/// fast path directly, so the slow lane never has to fire.
struct NoopLane;

impl TileFetchLane for NoopLane {
    fn enqueue(&self, _: &TileKey, _: TilePriority) {}
    fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
    fn attribution(&self) -> &str {
        ""
    }
    fn is_idle(&self) -> bool {
        true
    }
}

fn bench_disk_read_only(c: &mut Criterion) {
    let dir = BenchDir::new("disk-read");
    let key = TileKey::new(14, 14552, 6451);
    disk::write_disk(&dir.0, &key, SAMPLE);

    c.bench_function("disk_read_only", |b| {
        b.iter(|| {
            let bytes = disk::read_disk(black_box(&dir.0), black_box(&key));
            black_box(bytes)
        });
    });
}

fn bench_disk_read_then_decode(c: &mut Criterion) {
    let dir = BenchDir::new("disk-read-decode");
    let key = TileKey::new(14, 14552, 6451);
    disk::write_disk(&dir.0, &key, SAMPLE);

    c.bench_function("disk_read_then_decode", |b| {
        b.iter(|| {
            let bytes = disk::read_disk(&dir.0, &key).expect("planted");
            let decoded = decode::decode(&bytes);
            black_box(decoded)
        });
    });
}

/// End-to-end through the decoder lane: send bytes via `bytes_tx`,
/// receive a `DecodedTile` on `decoded_rx`. This isolates the cost
/// of the channel + decoder thread on top of bare `decode::decode`.
/// The render thread's poll cycle is **not** measured here — that's
/// up to 50 ms of jitter on top, addressed separately by issue #62.
fn bench_decoder_pipeline(c: &mut Criterion) {
    let (bytes_tx, bytes_rx) = mpsc::channel();
    let (decoded_rx, _decoder_handle) = decoder::spawn_decoder(bytes_rx);

    c.bench_function("decoder_pipeline", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for i in 0..iters {
                let k = TileKey::new(14, i as i32, 0);
                bytes_tx
                    .send((k.clone(), SAMPLE.to_vec()))
                    .expect("decoder thread alive");
                let (got, _) = decoded_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("decoder produced no result");
                assert_eq!(got, k);
            }
            start.elapsed()
        });
    });
}

/// `TileCache::get_tile` for an in-memory tile. This is the steady-
/// state hot path once a region is warmed up — should be O(1) on
/// `LruCache`.
fn bench_memory_hit(c: &mut Criterion) {
    let dir = BenchDir::new("mem-hit");
    let (_decoded_tx, decoded_rx) = mpsc::channel();
    let cache = TileCache::new(
        Box::new(NoopLane),
        decoded_rx,
        4096,
        Some(DiskFastPath {
            cache_dir: dir.0.clone(),
        }),
    );

    // Warm the LRU directly via the decoded channel emulation: feed
    // a single tile in by sending an empty `DecodedTile` through a
    // private side channel. We do this by constructing a tile in
    // memory using the standard pre-warm path: feed bytes through a
    // throwaway pipeline, then settle.
    //
    // Simplest trick: just plant the tile via the cache's own decoder
    // channel — but `decoded_rx` is owned by the cache. So we drive
    // the public API: insert via the slow lane... but NoopLane drops
    // requests. To keep the bench focused on pure memory-hit cost,
    // we cheat by re-binding the cache to a manual setup that lets
    // us seed an entry.
    //
    // Concretely: we plant nothing, accept that get_tile will
    // *first* miss and trigger the fast path (no disk file, no LRU
    // entry → returns None and enqueues), then on the second call
    // it's still a miss. We need an entry preloaded.
    //
    // Use a separate (tx, rx) pair as a bypass to seed the cache,
    // then swap out the rx. Simpler approach below using a re-built
    // cache that owns a side-channel sender.
    let key = TileKey::new(14, 14552, 6451);
    let (seed_tx, seed_rx) = mpsc::channel();
    let mut warm_cache = TileCache::new(Box::new(NoopLane), seed_rx, 4096, None);
    warm_cache.set_view(139.76, 35.68, 14);
    seed_tx.send((key.clone(), decode::decode(SAMPLE))).unwrap();
    warm_cache.poll_completed();
    drop(seed_tx);
    // `warm_cache` now has the tile in its LRU.
    let _ = cache; // silence unused-mut

    c.bench_function("memory_hit", |b| {
        b.iter(|| {
            // Don't return the borrow to avoid `FnMut` lifetime
            // tightening; just observe through `black_box(bool)`.
            let hit = warm_cache.get_tile(key.z, key.x, key.y).is_some();
            black_box(hit)
        });
    });
}

criterion_group!(
    benches,
    bench_disk_read_only,
    bench_disk_read_then_decode,
    bench_decoder_pipeline,
    bench_memory_hit,
);
criterion_main!(benches);
