//! Tile subsystem — fetching, caching, decoding, and view calculations.
//!
//! Responsibilities:
//!   cache.rs  — memory + disk storage, decode, stale detection, prefetch
//!   client.rs — HTTP fetch worker pool (raw bytes only)
//!   queue.rs  — LIFO request queue with size limit
//!   decode.rs — MVT protobuf decoding
//!   view.rs   — visible tile calculation

pub mod cache;
pub mod client;
pub mod decode;
pub mod queue;
pub mod view;

pub use cache::{TileCache, TileKey};
pub use decode::{DecodedTile, Feature, Point, TileLayer};
pub use view::{VisibleTile, visible_tiles};
