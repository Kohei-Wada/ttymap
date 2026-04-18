//! Tile subsystem — fetching, caching, decoding, and view calculations.
//!
//! Responsibilities:
//!   cache.rs  — memory + disk storage, decode, stale detection, prefetch
//!   fetch/    — backends that populate the cache (HTTP today, more later)
//!   decode.rs — MVT protobuf decoding
//!   view.rs   — visible tile calculation

pub mod cache;
pub mod decode;
pub mod fetch;
pub mod view;

pub use cache::{TileCache, TileKey};
pub use decode::{DecodedTile, Feature, Point, TileLayer};
pub use fetch::{HttpTileClient, TilePriority};
pub use view::{VisibleTile, visible_tiles};
