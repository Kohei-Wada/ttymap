//! Map frame — structured cell data produced by the render pipeline.
//! Widget implementation lives in ui/widget.rs.

use crate::geo::LonLat;

/// A single terminal cell in the rendered map.
#[derive(Clone, Debug)]
pub struct MapCell {
    pub ch: char,
    pub fg: u8,
    pub bg: u8,
}

/// A complete rendered map frame (row-major grid of cells). `center` and
/// `zoom` record the view the frame was rendered at so overlays (wiki
/// markers, etc.) can project points to the same coordinates regardless
/// of how stale the frame is relative to the current app state.
#[derive(Clone, Debug)]
pub struct MapFrame {
    pub cells: Vec<MapCell>,
    pub cols: u16,
    pub rows: u16,
    pub center: LonLat,
    pub zoom: f64,
}
