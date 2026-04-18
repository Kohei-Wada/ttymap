//! Fetch-queue priority for tile requests.
//!
//! `zoom_diff` dominates by declaration order: any in-range-zoom tile is
//! served before any stale-zoom tile regardless of distance. Within the
//! same `zoom_diff`, closer tiles win.

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct TilePriority {
    pub zoom_diff: u32,
    pub distance_sq: f64,
}
