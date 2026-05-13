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

use serde::{Deserialize, Serialize};

use crate::geo::{LonLat, base_zoom, ll2tile, tile_size_at_zoom};

/// One polyline submitted by a Lua plugin during a frame's `on_tick`
/// pass. Re-pushed every frame ("ephemeral re-submit"); the App-side
/// sink is drained into a `Vec<UserPolyline>` before each
/// `RenderTask::Draw` send.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Split a polyline at antimeridian crossings (|dlon| > 180 between
/// consecutive points). Each crossing inserts a pair of wrap points
/// at `lon = ±180` with linearly-interpolated lat, ending one sub-
/// polyline and starting the next. The result is a sequence of one
/// or more sub-polylines; each has all consecutive `|dlon| ≤ 180`,
/// which keeps the per-segment projection well-behaved.
///
/// A 2-point polyline that does not cross the antimeridian returns
/// a single sub-polyline equal to its input.
pub fn split_antimeridian(coords: &[LonLat]) -> Vec<Vec<LonLat>> {
    if coords.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut current: Vec<LonLat> = vec![coords[0]];
    for ll in &coords[1..] {
        let prev = *current.last().unwrap();
        let dlon = ll.lon - prev.lon;
        if dlon.abs() > 180.0 {
            // Antimeridian crossing on the shorter arc.
            //
            // Two cases:
            // - dlon < 0: prev is east of ll going the long way, so
            //   the shorter path goes east from prev, wrapping at
            //   +180 and re-entering at -180.
            // - dlon > 0: shorter path goes west from prev, wrapping
            //   at -180 and re-entering at +180.
            let (wrap_a, wrap_b, dist_to_wrap) = if dlon < 0.0 {
                (180.0_f64, -180.0_f64, 180.0 - prev.lon)
            } else {
                (-180.0_f64, 180.0_f64, prev.lon - (-180.0))
            };
            let total_short = 360.0 - dlon.abs();
            if total_short > f64::EPSILON {
                let t = dist_to_wrap / total_short;
                let lat_at_wrap = prev.lat + (ll.lat - prev.lat) * t;
                current.push(LonLat {
                    lon: wrap_a,
                    lat: lat_at_wrap,
                });
                result.push(std::mem::take(&mut current));
                current.push(LonLat {
                    lon: wrap_b,
                    lat: lat_at_wrap,
                });
            }
        }
        current.push(*ll);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Project a polyline into canvas-subpixel space with continuity:
/// the first coord uses the centre-aware shortest-modular projection
/// from [`ll_to_subpixel`]; each subsequent coord is projected
/// relative to the previous coord's subpixel x, picking the
/// representative within `±grid_subpixels/2` of `prev_x`. This keeps
/// adjacent polyline points "next to each other" in subpixel space
/// even when the modular wrap would otherwise place one on the far
/// left and the other on the far right of the canvas.
///
/// Y is unaffected by the wrap math; we just compute it once per
/// point as `canvas_h/2 + (pt.y - centre.y) * tile_size`.
///
/// Use after [`split_antimeridian`] so each sub-polyline has well-
/// behaved `|dlon| ≤ 180` segments going in.
pub fn project_polyline_continuous(
    coords: &[LonLat],
    center: LonLat,
    zoom: f64,
    canvas_w: usize,
    canvas_h: usize,
) -> Vec<(i32, i32)> {
    if coords.is_empty() {
        return Vec::new();
    }
    let z = base_zoom(zoom);
    let tile_size = tile_size_at_zoom(zoom);
    let grid_tiles = (1u64 << z) as f64;
    let grid_pixels = grid_tiles * tile_size;
    let center_t = ll2tile(center.lon, center.lat, z);
    let canvas_w_half = canvas_w as f64 / 2.0;
    let canvas_h_half = canvas_h as f64 / 2.0;

    let mut out = Vec::with_capacity(coords.len());

    // First point: classic centre-aware shortest path.
    let first = coords[0];
    let pt0 = ll2tile(first.lon, first.lat, z);
    let raw_dx0 = pt0.x - center_t.x;
    let dx0 = if grid_tiles > 0.0 && raw_dx0.abs() > grid_tiles / 2.0 {
        raw_dx0 - raw_dx0.signum() * grid_tiles
    } else {
        raw_dx0
    };
    let mut prev_dx_pixels = dx0 * tile_size;
    let py0 = canvas_h_half + (pt0.y - center_t.y) * tile_size;
    out.push(((canvas_w_half + prev_dx_pixels) as i32, py0 as i32));

    for ll in &coords[1..] {
        let pt = ll2tile(ll.lon, ll.lat, z);
        let raw_dx_pixels = (pt.x - center_t.x) * tile_size;
        // Adjust by integer multiples of grid_pixels so we land
        // within ±grid_pixels/2 of prev_dx_pixels.
        let mut adjusted = raw_dx_pixels;
        if grid_pixels > 0.0 {
            while adjusted - prev_dx_pixels > grid_pixels / 2.0 {
                adjusted -= grid_pixels;
            }
            while adjusted - prev_dx_pixels < -grid_pixels / 2.0 {
                adjusted += grid_pixels;
            }
        }
        let py = canvas_h_half + (pt.y - center_t.y) * tile_size;
        out.push(((canvas_w_half + adjusted) as i32, py as i32));
        prev_dx_pixels = adjusted;
    }

    out
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

    #[test]
    fn split_antimeridian_two_point_no_cross() {
        let coords = vec![
            LonLat {
                lon: 10.0,
                lat: 0.0,
            },
            LonLat {
                lon: 50.0,
                lat: 0.0,
            },
        ];
        let split = split_antimeridian(&coords);
        assert_eq!(split.len(), 1, "no crossing → single sub-polyline");
        assert_eq!(split[0].len(), 2);
    }

    #[test]
    fn split_antimeridian_tokyo_to_ny_splits_at_pacific() {
        let coords = vec![
            LonLat {
                lon: 139.76,
                lat: 35.68,
            }, // Tokyo
            LonLat {
                lon: -74.01,
                lat: 40.71,
            }, // NY
        ];
        let split = split_antimeridian(&coords);
        assert_eq!(split.len(), 2, "Tokyo→NY shorter arc crosses antimeridian");
        assert_eq!(split[0].len(), 2, "first sub-polyline has src + wrap_a");
        assert_eq!(split[1].len(), 2, "second sub-polyline has wrap_b + dst");
        // First sub-poly ends at +180 (going east from Tokyo).
        assert!((split[0][1].lon - 180.0).abs() < 1e-9);
        // Second sub-poly starts at -180.
        assert!((split[1][0].lon - (-180.0)).abs() < 1e-9);
        // Both wrap points share the same lat (linearly interpolated).
        assert!((split[0][1].lat - split[1][0].lat).abs() < 1e-9);
        // Lat at wrap is between Tokyo and NY's lats.
        let lat = split[0][1].lat;
        assert!(lat > 35.68 && lat < 40.71);
    }

    #[test]
    fn split_antimeridian_west_to_east_wraps_at_minus_180() {
        let coords = vec![
            LonLat {
                lon: -150.0,
                lat: 0.0,
            },
            LonLat {
                lon: 150.0,
                lat: 0.0,
            },
        ];
        let split = split_antimeridian(&coords);
        assert_eq!(split.len(), 2);
        // dlon = +300, shorter goes west via -180.
        assert!((split[0][1].lon - (-180.0)).abs() < 1e-9);
        assert!((split[1][0].lon - 180.0).abs() < 1e-9);
    }

    #[test]
    fn project_continuous_keeps_adjacent_points_close() {
        // Antimeridian-centred view (lon=180). Tokyo on the wrap-east
        // side, a point just past -180 lon on the wrap-west side. With
        // standard `ll_to_subpixel`, both project independently to the
        // shortest mod path; with `project_polyline_continuous`, the
        // second point is anchored to the first.
        let center = LonLat {
            lon: 180.0,
            lat: 0.0,
        };
        // Pick z=2 so grid_tiles = 4 and tile_size_at_zoom(2.0) = 256.
        let coords = vec![
            LonLat {
                lon: 170.0,
                lat: 0.0,
            },
            LonLat {
                lon: -170.0,
                lat: 0.0,
            }, // 20° apart on the shorter arc
        ];
        let pts = project_polyline_continuous(coords.as_slice(), center, 2.0, 800, 200);
        assert_eq!(pts.len(), 2);
        let (x0, _) = pts[0];
        let (x1, _) = pts[1];
        // Points are ~20° apart in lon → small subpixel separation.
        assert!(
            (x1 - x0).abs() < 200,
            "anchored projection keeps adjacent points within a few cells \
             of each other; got x0={x0}, x1={x1}"
        );
    }
}
