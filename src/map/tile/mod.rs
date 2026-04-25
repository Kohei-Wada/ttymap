//! Tile subsystem — fetching, caching, decoding, and view calculations.
//!
//! Responsibilities:
//!   key.rs       — `TileKey` (z, x, y) universal address
//!   cache.rs     — memory + disk storage, decode, stale detection, prefetch
//!   fetch/       — backends that populate the cache (HTTP today, more later)
//!   decode.rs    — MVT protobuf decoding
//!   property.rs  — feature property value type and accessors

pub mod cache;
pub mod decode;
pub mod fetch;
pub mod key;
pub mod property;

pub use cache::TileCache;
pub use decode::Feature;
pub use key::TileKey;
pub use property::PropertyValue;
