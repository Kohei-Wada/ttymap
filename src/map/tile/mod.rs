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
//!   cache.rs     — memory LRU + view state + prefetch (orchestrator)
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
