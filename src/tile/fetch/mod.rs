//! Tile fetch subsystem — backends that populate the cache.
//!
//! Today the only backend is `http` (MVT over HTTP, e.g. mapscii.me).
//! Future backends (mbtiles, offline bundles, mocked sources for tests)
//! go here as additional modules. When a second implementation arrives,
//! lift a `TileClient` trait into this `mod.rs` and let `cache` depend
//! on `Box<dyn TileClient>` instead of the concrete HTTP type.

pub mod http;
pub mod priority;
pub mod queue;

pub use http::HttpTileClient;
pub use priority::TilePriority;
