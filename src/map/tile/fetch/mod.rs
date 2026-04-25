//! Tile fetch subsystem — backends that populate the cache.
//!
//! Today the only backend is `http` (MVT over HTTP, default base
//! `mapscii.me`). Additional backends (mbtiles, pmtiles, local dirs,
//! mocks for tests) plug in as new modules whose entry type
//! `impl TileClient`. When a second backend lands, route selection
//! (from config or from the file extension of a user-supplied path)
//! lives here.

pub mod http;
pub mod priority;
pub mod queue;

pub use http::HttpTileClient;
pub use priority::TilePriority;

use crate::map::tile::key::TileKey;
use queue::PriorityFn;

/// Abstract tile-fetch backend. Cache owns a `Box<dyn TileClient>` and
/// interacts solely through these methods.
pub trait TileClient: Send + Sync {
    /// Enqueue a tile for fetching. Implementations dedup against
    /// in-flight / already-queued work.
    fn enqueue(&self, key: &TileKey, priority: TilePriority);

    /// Recompute queue priorities (typically after a viewport change).
    fn update_view(&self, priority_fn: &dyn PriorityFn<TileKey, TilePriority>);

    /// Attribution string for this source. Rendered by the attribution
    /// overlay (#42). OSM-derived sources return
    /// `"© OpenStreetMap contributors"`.
    fn attribution(&self) -> &str;

    /// Whether the client has finished all outstanding fetches.
    /// Returns `true` when no tiles are queued **and** no tiles are
    /// in-flight. Used by headless callers (`ttymap snap`) to decide
    /// when a frame is safe to commit.
    fn is_idle(&self) -> bool;
}
