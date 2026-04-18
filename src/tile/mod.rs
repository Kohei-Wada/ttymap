//! Tile subsystem — fetching, caching, decoding, and view calculations.
//!
//! Responsibilities:
//!   cache.rs  — memory + disk storage, decode, stale detection, prefetch
//!   fetch/    — backends that populate the cache (HTTP today, more later)
//!   decode.rs — MVT protobuf decoding

pub mod cache;
pub mod decode;
pub mod fetch;

pub use cache::TileCache;
pub use decode::Feature;

/// Build a `TileCache` wired to the default tile backend
/// (`MapsciiTileClient`). Hides the `mpsc` channel pairing from
/// callers.
pub fn build_tile_cache(enable_disk_cache: bool) -> TileCache {
    let (tx, rx) = std::sync::mpsc::channel();
    let client: Box<dyn fetch::TileClient> = Box::new(fetch::MapsciiTileClient::new(tx));
    TileCache::new(client, rx, enable_disk_cache)
}
