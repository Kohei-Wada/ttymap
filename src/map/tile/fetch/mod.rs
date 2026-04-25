//! Tile fetch subsystem.
//!
//! Two trait layers, deliberately separate:
//!
//! - [`TileFetcher`] — small per-backend trait. Given a `TileKey`,
//!   return bytes (or an error). Backends only implement this; they
//!   don't see queues, workers, in-flight sets, or priority logic.
//! - [`TileFetchLane`] — facade the orchestrator (`TileCache`) sees.
//!   Exposes `enqueue` / `update_view` / `is_idle` / `attribution`.
//!   The generic [`lane::FetchLane<F>`] wraps any `TileFetcher` and
//!   provides this interface for free, so adding a new backend is
//!   just one [`TileFetcher`] impl.
//!
//! Today the only backend is [`http::HttpFetcher`] (MVT over HTTP,
//! default base `mapscii.me`). When a second backend lands (mbtiles,
//! pmtiles, local dirs), route selection (from config or from the
//! file extension of a user-supplied path) lives here.

pub mod http;
pub mod lane;
pub mod priority;
pub mod queue;

pub use http::HttpFetcher;
pub use lane::FetchLane;
pub use priority::TilePriority;

use std::fmt;

use queue::PriorityFn;

use crate::map::tile::key::TileKey;

/// Per-backend trait — "given a key, return bytes". Stays minimal so
/// new backends only deal with their own protocol; the queue,
/// concurrency, dedup, and priority machinery lives in
/// [`FetchLane`].
pub trait TileFetcher: Send + Sync {
    /// Fetch the bytes of `key`. Backends are free to do disk-cache
    /// reads, HTTP, sqlite queries, etc. The returned bytes are
    /// forwarded raw to the decoder.
    fn fetch(&self, key: &TileKey) -> Result<Vec<u8>, FetchError>;

    /// Static attribution string for this source — shown by the
    /// attribution overlay. OSM-derived sources return
    /// `"© OpenStreetMap contributors"`.
    fn attribution(&self) -> &str;
}

/// Facade trait the orchestrator consumes via `Box<dyn TileFetchLane>`.
/// Implemented generically by `FetchLane<F>` for any
/// `F: TileFetcher`.
pub trait TileFetchLane: Send + Sync {
    /// Enqueue a tile for fetching. Implementations dedup against
    /// in-flight / already-queued work.
    fn enqueue(&self, key: &TileKey, priority: TilePriority);

    /// Recompute queue priorities (typically after a viewport change).
    /// Backends without a meaningful priority order can no-op.
    fn update_view(&self, priority_fn: &dyn PriorityFn<TileKey, TilePriority>);

    /// Static attribution string — typically delegated to the wrapped
    /// `TileFetcher`.
    fn attribution(&self) -> &str;

    /// Whether the lane has finished all outstanding fetches: queue
    /// empty **and** nothing in-flight. Used by headless callers
    /// (`ttymap snap`) to decide when a frame is safe to commit.
    fn is_idle(&self) -> bool;
}

/// Errors a `TileFetcher` may report. Intentionally a thin
/// `String`-carrying type — the worker logs the message and emits an
/// empty `Vec<u8>` on the result channel (negative cache), so we
/// don't need fine-grained variants today.
#[derive(Debug)]
pub struct FetchError {
    pub message: String,
}

impl FetchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}
