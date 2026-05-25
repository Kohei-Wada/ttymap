//! Tile cache (orchestrator) — memory LRU + view state.
//!
//! After the three-layer-pipe refactor, decoding is on its own thread
//! (`super::decoder`) and arrivals here are already `DecodedTile`s.
//!
//! In addition to that "slow path" (FetchLane → decoder → cache), the
//! cache holds an optional **synchronous disk fast path** for tiles
//! the user already has on disk. On a memory miss we read the file
//! directly on the render thread and push the bytes straight to the
//! decoder lane, **bypassing the worker queue entirely**. This avoids
//! the worker handoff latency and the queue-overflow drops that fast
//! pan / zoom would otherwise inflict on disk-resident tiles.
//!
//! Writes still go through the `TileFetcher` layer (specifically
//! `DiskCachedFetcher`'s write-through). The two readers (decorator
//! and this fast path) share the layout in `super::disk`.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::mpsc;

use log::debug;
use lru::LruCache;

use super::decode::DecodedTile;
use super::disk;
use super::fetch::{TileFetchLane, TilePriority};
use super::key::TileKey;

/// The render-thread fast path for disk-resident tiles. On a
/// memory miss the cache reads + decodes the file synchronously and
/// inserts into the LRU, so a disk hit lands in the same render
/// frame instead of paying a poll-cycle round trip through the
/// decoder lane.
pub struct DiskFastPath {
    pub cache_dir: PathBuf,
}

pub struct TileCache {
    client: Box<dyn TileFetchLane>,
    memory_cache: LruCache<TileKey, DecodedTile>,
    current_z: u32,
    center_x: f64,
    center_y: f64,
    rx: mpsc::Receiver<(TileKey, DecodedTile)>,
    disk_fast_path: Option<DiskFastPath>,
}

impl TileCache {
    /// Build a cache around an injected `TileFetchLane` and the
    /// receiving end of the **decoder** channel.
    ///
    /// `disk_fast_path` is optional: if disk caching is configured,
    /// the composition root passes a `DiskFastPath` that lets the
    /// cache short-circuit memory misses against disk on the render
    /// thread, sending the bytes straight to the decoder.
    pub fn new(
        client: Box<dyn TileFetchLane>,
        rx: mpsc::Receiver<(TileKey, DecodedTile)>,
        cache_size: usize,
        disk_fast_path: Option<DiskFastPath>,
    ) -> Self {
        // `cache_size` may legitimately be configured to 0 by paranoid
        // users; clamp to 1 since `NonZeroUsize::new` rejects zero.
        let capacity = NonZeroUsize::new(cache_size.max(1)).unwrap();
        TileCache {
            client,
            memory_cache: LruCache::new(capacity),
            current_z: 0,
            center_x: 0.0,
            center_y: 0.0,
            rx,
            disk_fast_path,
        }
    }

    /// Update view state. Reprioritizes any queued fetch entries.
    pub fn set_view(&mut self, center_lon: f64, center_lat: f64, z: u32) {
        self.current_z = z;
        let center = crate::geo::ll2tile(center_lon, center_lat, z);
        self.center_x = center.x;
        self.center_y = center.y;

        let cx = self.center_x;
        let cy = self.center_y;
        let cz = self.current_z;
        // Modular distance is computed in `cz`-space (matching cx/cy)
        // so the antimeridian wrap is correct for same-z keys. Cross-z
        // scoring is approximate but `zoom_diff` already dominates the
        // composite priority.
        let grid_size = crate::geo::tile_grid_size(cz);

        self.client.update_view(&|key: &TileKey| TilePriority {
            zoom_diff: key.z.abs_diff(cz),
            distance_sq: crate::geo::tile_distance_sq(key.x, key.y, cx, cy, grid_size),
        });
    }

    /// Whether the fetch backend has finished all outstanding work.
    /// `true` means the queue is empty and no tiles are in-flight —
    /// there will be no more tile arrivals without a new request.
    pub fn is_fetch_idle(&self) -> bool {
        self.client.is_idle()
    }

    /// Active tile backend's attribution string (typically
    /// "© OpenStreetMap …"). `None` when the backend has nothing to
    /// display. Single source of truth so `App` and the Lua bridge
    /// don't fork the lookup against the inner fetcher's value
    /// directly.
    pub fn attribution(&self) -> Option<String> {
        let s = self.client.attribution();
        (!s.is_empty()).then(|| s.to_string())
    }

    /// Drain decoded-tile arrivals into the memory LRU. Returns true
    /// if any current-zoom tile arrived (i.e. the render thread
    /// should redraw).
    pub fn poll_completed(&mut self) -> bool {
        let mut any_new = false;
        while let Ok((key, decoded)) = self.rx.try_recv() {
            let is_current = key.z.abs_diff(self.current_z) <= 1;
            if is_current {
                debug!("poll_completed: {} ({} layers)", key, decoded.layers.len());
                any_new = true;
            }
            // `LruCache::put` evicts the least-recently-used entry
            // when at capacity and treats the inserted key as MRU.
            self.memory_cache.put(key, decoded);
        }
        any_new
    }

    /// Get a tile.
    ///
    /// 1. Memory LRU hit → return immediately, bumping to MRU.
    /// 2. **Disk fast path** (if configured): synchronous file read
    ///    **and decode** on the render thread, then insert into
    ///    memory and return the reference. This restores the
    ///    pre-refactor "disk hit = same frame" behaviour: previously
    ///    a disk hit had to round-trip through the decoder thread +
    ///    one render poll cycle (≥25 ms latency). At ~320 µs decode
    ///    per tile, doing it on the render thread is cheaper than
    ///    the cross-thread hop for the common-case responsive path.
    /// 3. Otherwise enqueue the key on the fetch lane (HTTP
    ///    fetches still go through the decoder thread, since they
    ///    arrive at random times and would otherwise block the
    ///    render thread on each completion).
    pub fn get_tile(&mut self, z: u32, x: i32, y: i32) -> Option<&DecodedTile> {
        let key = TileKey::new(z, x, y);

        if self.memory_cache.contains(&key) {
            debug!("cache: memory hit {}", key);
            return self.memory_cache.get(&key);
        }

        if let Some(fast) = &self.disk_fast_path
            && let Some(bytes) = disk::read_disk(&fast.cache_dir, &key)
        {
            let decoded = super::decode::decode(&bytes);
            debug!(
                "cache: disk hit {} ({} bytes, {} layers)",
                key,
                bytes.len(),
                decoded.layers.len()
            );
            self.memory_cache.put(key.clone(), decoded);
            return self.memory_cache.get(&key);
        }

        let grid_size = crate::geo::tile_grid_size(self.current_z);
        let priority = TilePriority {
            zoom_diff: key.z.abs_diff(self.current_z),
            distance_sq: crate::geo::tile_distance_sq(
                key.x,
                key.y,
                self.center_x,
                self.center_y,
                grid_size,
            ),
        };
        self.client.enqueue(&key, priority);
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::fetch::queue::PriorityFn;
    use super::*;

    /// A no-op `TileFetchLane` that swallows everything. Suitable
    /// for tests that exercise cache state without checking dispatch.
    struct NoopLane;
    impl TileFetchLane for NoopLane {
        fn enqueue(&self, _: &TileKey, _: TilePriority) {}
        fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
        fn attribution(&self) -> &str {
            "noop"
        }
        fn is_idle(&self) -> bool {
            true
        }
    }

    /// Regression for issue #105. The doc comment / data-structure
    /// names promise LRU eviction, but the implementation used FIFO
    /// (insertion order) — a hot tile that was hit between inserts
    /// got evicted at the same time as cold ones. Under true LRU,
    /// recent access promotes the tile to MRU and protects it.
    #[test]
    fn lru_eviction_keeps_recently_accessed_tile() {
        // cache_size = 2. Inject A, B as empty `DecodedTile`s through
        // the decoder channel. Hit A (LRU bumps it to MRU). Insert C.
        // With LRU, B is evicted; A survives. With FIFO, A (oldest
        // insert) is evicted regardless of the hit.
        let (tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(Box::new(NoopLane), rx, 2, None);

        let a = TileKey::new(0, 0, 0);
        let b = TileKey::new(0, 1, 0);
        let c = TileKey::new(0, 2, 0);

        tx.send((a.clone(), DecodedTile::empty())).unwrap();
        tx.send((b.clone(), DecodedTile::empty())).unwrap();
        cache.poll_completed();

        // Access A: bumps to MRU under LRU.
        assert!(cache.get_tile(a.z, a.x, a.y).is_some());

        tx.send((c.clone(), DecodedTile::empty())).unwrap();
        cache.poll_completed();

        assert!(
            cache.get_tile(a.z, a.x, a.y).is_some(),
            "recently-accessed tile A must survive when C displaces \
             the LRU entry (B), not the most-recently-touched one"
        );
    }

    /// Proves `TileCache` drives its backend purely through the
    /// `TileFetchLane` trait: cache misses dispatch to the injected
    /// lane, with no HTTP or worker threads involved.
    #[test]
    fn test_cache_misses_dispatch_through_injected_client() {
        struct RecordingLane(Arc<Mutex<Vec<TileKey>>>);
        impl TileFetchLane for RecordingLane {
            fn enqueue(&self, key: &TileKey, _: TilePriority) {
                self.0.lock().unwrap().push(key.clone());
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                "mock"
            }
            fn is_idle(&self) -> bool {
                true
            }
        }

        let log = Arc::new(Mutex::new(Vec::<TileKey>::new()));
        let client: Box<dyn TileFetchLane> = Box::new(RecordingLane(log.clone()));
        let (_tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(client, rx, 64, None);

        cache.get_tile(3, 1, 2);
        cache.get_tile(3, 5, 6);

        assert_eq!(
            *log.lock().unwrap(),
            vec![TileKey::new(3, 1, 2), TileKey::new(3, 5, 6)],
        );
    }

    /// Memory hit short-circuits the lane: a tile already inserted
    /// via the decoder channel must not result in another `enqueue`.
    #[test]
    fn memory_hit_does_not_dispatch_to_lane() {
        struct CountingLane(Arc<Mutex<usize>>);
        impl TileFetchLane for CountingLane {
            fn enqueue(&self, _: &TileKey, _: TilePriority) {
                *self.0.lock().unwrap() += 1;
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                ""
            }
            fn is_idle(&self) -> bool {
                true
            }
        }

        let calls = Arc::new(Mutex::new(0usize));
        let client: Box<dyn TileFetchLane> = Box::new(CountingLane(calls.clone()));
        let (tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(client, rx, 4, None);

        let key = TileKey::new(2, 1, 1);
        tx.send((key.clone(), DecodedTile::empty())).unwrap();
        cache.poll_completed();

        for _ in 0..3 {
            assert!(cache.get_tile(key.z, key.x, key.y).is_some());
        }
        assert_eq!(
            *calls.lock().unwrap(),
            0,
            "memory-hit path must not enqueue"
        );
    }

    /// Disk fast path: a tile that's already on disk must be read,
    /// decoded, and inserted into the memory LRU **on the same
    /// `get_tile` call** — so the visible-tile path can use it
    /// immediately. Verified by sending no bytes through any
    /// channel: the cache resolves the tile entirely on the render
    /// thread.
    #[test]
    fn disk_fast_path_resolves_synchronously_on_hit() {
        use std::time::{SystemTime, UNIX_EPOCH};

        struct CountingLane(Arc<Mutex<usize>>);
        impl TileFetchLane for CountingLane {
            fn enqueue(&self, _: &TileKey, _: TilePriority) {
                *self.0.lock().unwrap() += 1;
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                ""
            }
            fn is_idle(&self) -> bool {
                true
            }
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ttymap-cache-fastpath-{}", nanos));
        std::fs::create_dir_all(&dir).unwrap();

        // Plant a (degenerate but parseable) tile: empty PBF body
        // decodes to `DecodedTile { layers: empty }`, which is
        // enough to verify the fast path inserted *something* into
        // the LRU.
        let key = TileKey::new(3, 1, 2);
        super::disk::write_disk(&dir, &key, b"");

        let enqueue_calls = Arc::new(Mutex::new(0usize));
        let client: Box<dyn TileFetchLane> = Box::new(CountingLane(enqueue_calls.clone()));

        let (_decoded_tx, decoded_rx) = mpsc::channel();
        let mut cache = TileCache::new(
            client,
            decoded_rx,
            4,
            Some(DiskFastPath {
                cache_dir: dir.clone(),
            }),
        );

        // Memory miss → fast path → returns the just-inserted tile.
        assert!(
            cache.get_tile(key.z, key.x, key.y).is_some(),
            "disk-resident tile must be available in the same call"
        );
        // No worker dispatch.
        assert_eq!(
            *enqueue_calls.lock().unwrap(),
            0,
            "disk hit must not dispatch to the worker queue"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Disk miss with `disk_fast_path = Some(...)` must fall through
    /// to the lane (the slow path), not silently drop the request.
    #[test]
    fn disk_fast_path_misses_fall_through_to_enqueue() {
        use std::time::{SystemTime, UNIX_EPOCH};

        struct RecordingLane(Arc<Mutex<Vec<TileKey>>>);
        impl TileFetchLane for RecordingLane {
            fn enqueue(&self, key: &TileKey, _: TilePriority) {
                self.0.lock().unwrap().push(key.clone());
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                ""
            }
            fn is_idle(&self) -> bool {
                true
            }
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ttymap-cache-fastpath-miss-{}", nanos));
        std::fs::create_dir_all(&dir).unwrap();

        let log = Arc::new(Mutex::new(Vec::new()));
        let client: Box<dyn TileFetchLane> = Box::new(RecordingLane(log.clone()));

        let (_decoded_tx, decoded_rx) = mpsc::channel();
        let mut cache = TileCache::new(
            client,
            decoded_rx,
            4,
            Some(DiskFastPath {
                cache_dir: dir.clone(),
            }),
        );

        let key = TileKey::new(3, 9, 9);
        assert!(cache.get_tile(key.z, key.x, key.y).is_none());

        let recorded = log.lock().unwrap().clone();
        assert_eq!(recorded, vec![key], "disk miss must reach the lane");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
