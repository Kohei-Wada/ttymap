//! User-overlay primitives for the render thread.
//!
//! `UserPolyline` is the value passed from `App` to the render thread
//! via `RenderTask::Draw { overlays, .. }`. `ll_to_subpixel` projects
//! a world coordinate into the canvas's subpixel grid (canvas units,
//! not cells), matching the coordinate space `Canvas::polyline`
//! expects.
//!
//! Subpixel coords differ from `MapProjection::ll_to_cell` only in
//! their unit: a cell spans 2 subpixels horizontally and 4 vertically,
//! so subpixel coords are exactly `(cell.x * 2, cell.y * 4)` for the
//! same point. We don't reuse `MapProjection` directly because that
//! type rounds to whole cells and clips against `cols`/`rows`, while
//! the polyline path needs raw `i32` subpixel coords (Bresenham handles
//! out-of-canvas writes via `BrailleBuffer::set_pixel`'s bounds check).

use crate::geo::{LonLat, base_zoom, ll2tile, tile_size_at_zoom};

/// One polyline submitted by a Lua plugin during a frame's `on_tick`
/// pass. Re-pushed every frame ("ephemeral re-submit"); the App-side
/// sink is drained into a `Vec<UserPolyline>` before each
/// `RenderTask::Draw` send.
#[derive(Debug, Clone)]
pub struct UserPolyline {
    /// World-space coordinates. Length-1 polylines are silently dropped
    /// at the Lua bridge before reaching this type.
    pub coords: Vec<LonLat>,
    /// xterm-256 palette index, the same colour unit `Canvas::polyline`
    /// already consumes.
    pub color: u8,
}

/// Project a world coordinate into canvas-subpixel `(x, y)`.
///
/// `canvas_w` / `canvas_h` are the renderer's pixel dimensions
/// (`cols * 2`, `rows * 4`). Output may fall outside `[0, canvas_w) ×
/// [0, canvas_h)` — the caller (Bresenham via `Canvas::polyline`)
/// takes care of clipping by relying on `BrailleBuffer::set_pixel`'s
/// bounds check.
pub fn ll_to_subpixel(
    ll: LonLat,
    center: LonLat,
    zoom: f64,
    canvas_w: usize,
    canvas_h: usize,
) -> (i32, i32) {
    let z = base_zoom(zoom);
    let tile_size = tile_size_at_zoom(zoom);
    let center_t = ll2tile(center.lon, center.lat, z);
    let pt = ll2tile(ll.lon, ll.lat, z);

    // Antimeridian wrap: pick the shorter modular path on x so a point
    // on the wrap side projects right next to the centre instead of
    // way off-screen. Mirrors the same fix in `MapProjection::ll_to_cell`
    // (issue #96).
    let grid = (1u64 << z) as f64;
    let raw_dx = pt.x - center_t.x;
    let dx = if grid > 0.0 && raw_dx.abs() > grid / 2.0 {
        raw_dx - raw_dx.signum() * grid
    } else {
        raw_dx
    };

    let px = canvas_w as f64 / 2.0 + dx * tile_size;
    let py = canvas_h as f64 / 2.0 + (pt.y - center_t.y) * tile_size;
    (px as i32, py as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_projects_to_canvas_centre() {
        let center = LonLat {
            lon: 13.42,
            lat: 52.51,
        };
        let (x, y) = ll_to_subpixel(center, center, 10.0, 200, 80);
        // (canvas_w / 2, canvas_h / 2) — exact centre.
        assert_eq!(x, 100);
        assert_eq!(y, 40);
    }

    #[test]
    fn doubling_zoom_doubles_offset() {
        let center = LonLat { lon: 0.0, lat: 0.0 };
        let east = LonLat { lon: 1.0, lat: 0.0 };
        let (x_low, _) = ll_to_subpixel(east, center, 5.0, 1000, 1000);
        let (x_high, _) = ll_to_subpixel(east, center, 6.0, 1000, 1000);
        let off_low = x_low - 500;
        let off_high = x_high - 500;
        // tile_size doubles when the integer zoom advances by 1.
        // Allow ±1 slack for as-i32 truncation.
        assert!(
            (off_high - 2 * off_low).abs() <= 1,
            "expected ~doubled offset, got {off_low} → {off_high}"
        );
    }

    #[test]
    fn antimeridian_wrap_keeps_point_near_centre() {
        // Centre near +180, point near -180 (wrap-side).
        let center = LonLat {
            lon: 179.9,
            lat: 0.0,
        };
        let near = LonLat {
            lon: -179.9,
            lat: 0.0,
        };
        let canvas_w = 800;
        let (x, _) = ll_to_subpixel(near, center, 5.0, canvas_w, 200);
        // Should land within ~5% of the canvas width from the centre,
        // not way off-screen.
        let off = (x - canvas_w as i32 / 2).abs();
        assert!(
            off < canvas_w as i32 / 20,
            "expected near centre, got x={x}"
        );
    }
}
