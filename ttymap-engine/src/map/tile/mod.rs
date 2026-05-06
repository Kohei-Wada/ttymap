//! Tile subsystem — fetching, decoding, and caching.
//!
//! Three-layer pipeline (a tile flows left → right):
//!
//! ```text
//!   fetch::FetchLane<F>     decoder      cache::TileCache
//!   ───────────────────     ───────      ────────────────
//!   bytes from F (HTTP,     decode()     LRU memory store
//!   disk, mbtiles, …)
//! ```
//!
//! Modules:
//!   key.rs       — `TileKey` (z, x, y) universal address
//!   property.rs  — feature property value type and accessors
//!   decode/      — pure protobuf → `DecodedTile` (geometry / tags / decompress)
//!   decoder.rs   — relay thread: bytes → `DecodedTile`, off the render thread
//!   cache.rs     — memory LRU + view state (orchestrator)
//!   fetch/       — backends that produce bytes (HTTP today, more later);
//!                  `TileFetcher` is per-backend, `FetchLane<F>` is generic
//!                  queue / workers / dedup / priority

pub mod cache;
pub mod decode;
pub mod decoder;
pub mod disk;
pub mod fetch;
pub mod key;
pub mod property;

pub use cache::TileCache;
pub use decode::Feature;
pub use key::TileKey;
pub use property::PropertyValue;

use crate::config::Config;

/// Compose the tile subsystem from `config` and return the live
/// [`TileCache`] plus the decoder's `wake_rx` (used by the render
/// thread to know when a freshly-decoded tile arrived).
///
/// Wires the three-layer pipeline plus the optional disk fast path:
///
/// ```text
///                   ┌── render-thread disk fast path ──────────────────┐
///                   ▼                                                   │
///   FetchLane<F>  ──bytes──▶  decoder thread  ──DecodedTile──▶  TileCache
/// ```
///
/// where `F` is `DiskCachedFetcher<HttpFetcher>` when disk cache is
/// enabled, else just `HttpFetcher`. The fast path lets `TileCache`
/// read disk synchronously and push bytes directly to the decoder,
/// skipping the worker queue.
///
/// Backend dispatch happens here: a future MBTiles / PMTiles backend
/// would pick a different `TileFetcher`. `FetchLane` provides queue
/// / dedup / priority for any of them; `decoder::spawn_decoder` and
/// the cache are backend-agnostic.
pub fn build(
    config: &Config,
) -> Result<(TileCache, crossbeam_channel::Receiver<()>), crate::EngineError> {
    use directories::ProjectDirs;
    use std::fs;

    use crate::map::tile::cache::DiskFastPath;
    use crate::map::tile::decoder;
    use crate::map::tile::fetch::{DiskCachedFetcher, FetchLane, HttpFetcher, TileFetchLane};

    /// Worker count for the HTTP backend. HTTP is I/O-bound, so a
    /// small pool covers the typical visible-tile + prefetch fan-out
    /// without saturating the upstream.
    const HTTP_WORKERS: usize = 6;

    let cache_dir = if config.cache.tiles {
        match ProjectDirs::from("", "", "ttymap") {
            Some(proj_dirs) => {
                let dir = proj_dirs.cache_dir().to_path_buf();
                fs::create_dir_all(&dir).map_err(|source| crate::EngineError::CacheDir {
                    path: dir.clone(),
                    source,
                })?;
                Some(dir)
            }
            None => None,
        }
    } else {
        None
    };

    let (bytes_tx, bytes_rx) = std::sync::mpsc::channel();
    let http = HttpFetcher::new()?;

    // The lane wraps an HTTP fetcher; if disk cache is enabled, layer
    // a `DiskCachedFetcher` decorator on top so worker-side hits
    // short-circuit the network and on miss we write through.
    let lane: Box<dyn TileFetchLane> = match cache_dir.clone() {
        Some(dir) => Box::new(FetchLane::new(
            DiskCachedFetcher::new(http, dir),
            HTTP_WORKERS,
            bytes_tx.clone(),
        )),
        None => Box::new(FetchLane::new(http, HTTP_WORKERS, bytes_tx.clone())),
    };

    let (decoded_rx, wake_rx, _decoder_handle) = decoder::spawn_decoder(bytes_rx);

    // The render-thread fast path: on a memory miss the cache reads
    // and decodes the file synchronously, putting the tile into the
    // LRU in the same render frame. This restores pre-refactor disk-
    // hit responsiveness; HTTP fetches still flow through the slow
    // lane below.
    let disk_fast_path = cache_dir.map(|cache_dir| DiskFastPath { cache_dir });

    Ok((
        TileCache::new(lane, decoded_rx, config.cache.memory_tiles, disk_fast_path),
        wake_rx,
    ))
}
