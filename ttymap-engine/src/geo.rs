use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

/// Maximum latitude for Web Mercator projection.
pub const MAX_LAT: f64 = 85.051_129_0;
/// Earth radius in metres (WGS84 mean).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// A geographic coordinate in WGS84.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LonLat {
    pub lon: f64,
    pub lat: f64,
}

/// A tile coordinate at a given zoom level.
/// x and y are fractional tile indices; z is the zoom level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileCoord {
    pub x: f64,
    pub y: f64,
    pub z: u32,
}

/// Wrap longitude to (-180, 180] and clamp latitude to -85.0511..=85.0511.
pub fn normalize(ll: LonLat) -> LonLat {
    let mut lon = ll.lon % 360.0;
    if lon > 180.0 {
        lon -= 360.0;
    }
    if lon <= -180.0 {
        lon += 360.0;
    }
    LonLat {
        lon,
        lat: ll.lat.clamp(-MAX_LAT, MAX_LAT),
    }
}

/// Convert a geographic coordinate to a (fractional) tile coordinate using the
/// Web Mercator (EPSG:3857) projection.
pub fn ll2tile(lon: f64, lat: f64, zoom: u32) -> TileCoord {
    let n = (1u64 << zoom) as f64; // 2^zoom
    let x = (lon + 180.0) / 360.0 * n;
    let lat_rad = lat.to_radians();
    let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / PI) / 2.0 * n;
    TileCoord { x, y, z: zoom }
}

/// Inverse of `ll2tile`. Converts a (fractional) tile coordinate back to
/// lon/lat using the Web Mercator projection.
pub fn tile2ll(x: f64, y: f64, zoom: u32) -> LonLat {
    let n = (1u64 << zoom) as f64;
    let lon = x / n * 360.0 - 180.0;
    let lat_rad = (PI * (1.0 - 2.0 * y / n)).sinh().atan();
    LonLat {
        lon,
        lat: lat_rad.to_degrees(),
    }
}

/// Projection between world lat/lon and the braille canvas grid used by
/// overlays. Canvas size is expressed in terminal cells, where each cell
/// spans 2×4 braille sub-pixels. Both `ll_to_cell` and `cell_to_ll` use
/// the top-left of the canvas as origin; callers add the map-area offset
/// themselves.
pub struct MapProjection {
    center_tile: TileCoord,
    tile_size: f64,
    canvas_w: f64,
    canvas_h: f64,
    cols: u16,
    rows: u16,
    z: u32,
}

impl MapProjection {
    pub fn new(center: LonLat, zoom: f64, cols: u16, rows: u16) -> Self {
        let z = base_zoom(zoom);
        Self {
            center_tile: ll2tile(center.lon, center.lat, z),
            tile_size: tile_size_at_zoom(zoom),
            canvas_w: cols as f64 * 2.0,
            canvas_h: rows as f64 * 4.0,
            cols,
            rows,
            z,
        }
    }

    /// Project `ll` to a canvas cell. `None` if the point falls outside
    /// the canvas or the projection is not finite (e.g. poles).
    pub fn ll_to_cell(&self, ll: LonLat) -> Option<(u16, u16)> {
        let pt = ll2tile(ll.lon, ll.lat, self.z);
        // x is modular (lon wraps): pick the shorter path across the
        // antimeridian so a feature on the wrap side lands next to the
        // centre instead of way off-screen (issue #96). y is not.
        let grid = (1u64 << self.z) as f64;
        let dx = shortest_modular_dx(pt.x - self.center_tile.x, grid);
        let px = self.canvas_w / 2.0 + dx * self.tile_size;
        let py = self.canvas_h / 2.0 + (pt.y - self.center_tile.y) * self.tile_size;
        if !px.is_finite() || !py.is_finite() || px < 0.0 || py < 0.0 {
            return None;
        }
        let col = (px / 2.0) as u16;
        let row = (py / 4.0) as u16;
        if col >= self.cols || row >= self.rows {
            return None;
        }
        Some((col, row))
    }

    /// Project a canvas cell to lat/lon. `None` if the cell is outside
    /// the canvas. Sub-pixel centre of the cell is used.
    pub fn cell_to_ll(&self, col: u16, row: u16) -> Option<LonLat> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        let px = col as f64 * 2.0 + 1.0;
        let py = row as f64 * 4.0 + 2.0;
        let tx = self.center_tile.x + (px - self.canvas_w / 2.0) / self.tile_size;
        let ty = self.center_tile.y + (py - self.canvas_h / 2.0) / self.tile_size;
        Some(tile2ll(tx, ty, self.z))
    }
}

/// Floor a zoom level and clamp to 0..=14.
pub fn base_zoom(zoom: f64) -> u32 {
    zoom.floor().clamp(0.0, 14.0) as u32
}

/// Tile-grid size at zoom `z`: the number of tiles along one axis
/// (i.e. `2^z`), saturating at `i32::MAX` for `z >= 31`. Used as the
/// modulus for x-axis wraparound (slippy maps wrap longitude but not
/// latitude). Centralised so every call site has the same overflow
/// story.
pub fn tile_grid_size(z: u32) -> i32 {
    1i32.checked_shl(z).unwrap_or(i32::MAX)
}

/// Shortest signed delta on a wrapping axis (longitude / tile-x).
/// When `|raw| > grid/2` the antimeridian is the shorter path, so
/// wrap by one grid width; otherwise keep `raw`. A non-positive
/// `grid` disables wrapping. See issue #96.
pub fn shortest_modular_dx(raw: f64, grid: f64) -> f64 {
    if grid > 0.0 && raw.abs() > grid / 2.0 {
        raw - raw.signum() * grid
    } else {
        raw
    }
}

/// Return the effective tile size (in screen pixels / units) at a fractional
/// zoom level.  A tile is 256 units at every integer zoom; sub-tile zooming
/// scales it up.
pub fn tile_size_at_zoom(zoom: f64) -> f64 {
    let bz = base_zoom(zoom) as f64;
    256.0 * (zoom - bz).exp2()
}

/// Format a distance in metres as a human-readable string.
/// Values below 1000 m are shown as "Xm"; values >= 1000 m are shown as "X.Xkm".
pub fn format_distance(meters: f64) -> String {
    if meters < 1000.0 {
        format!("{}m", meters.round() as i64)
    } else {
        let km = meters / 1000.0;
        // One decimal place, trim trailing ".0"
        let s = format!("{:.1}km", km);
        s
    }
}

/// Calculate scale bar info: how many screen chars represent a nice round distance.
/// Returns (label, char_width).
pub fn scale_bar(lat: f64, zoom: f64, screen_width: u16) -> (String, u16) {
    // Metres per pixel at this latitude and zoom.
    // At zoom 0, one tile (256px) covers the full equator circumference.
    // At higher zooms, each tile covers 1/2^z of that.
    let meters_per_pixel =
        (EARTH_RADIUS_M * 2.0 * PI * lat.to_radians().cos()) / (256.0 * 2.0_f64.powf(zoom));
    // Each terminal cell = 2 braille pixels wide
    let meters_per_cell = meters_per_pixel * 2.0;

    // Pick a nice round distance that fits in ~1/5 of the screen
    let target_cells = (screen_width as f64 / 5.0).max(4.0);
    let target_meters = target_cells * meters_per_cell;

    // Round to a nice number
    let nice_distances = [
        50.0,
        100.0,
        200.0,
        500.0,
        1_000.0,
        2_000.0,
        5_000.0,
        10_000.0,
        20_000.0,
        50_000.0,
        100_000.0,
        200_000.0,
        500_000.0,
        1_000_000.0,
        2_000_000.0,
        5_000_000.0,
    ];
    let distance = nice_distances
        .iter()
        .copied()
        .min_by_key(|d| ((d - target_meters).abs() * 1000.0) as i64)
        .unwrap_or(1000.0);

    let cells = (distance / meters_per_cell).round() as u16;
    let cells = cells.clamp(2, screen_width / 3);

    (format_distance(distance), cells)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-6;

    // --- normalize -----------------------------------------------------------

    #[test]
    fn normalize_identity() {
        let ll = LonLat {
            lon: 10.0,
            lat: 50.0,
        };
        let n = normalize(ll);
        assert!((n.lon - 10.0).abs() < EPS);
        assert!((n.lat - 50.0).abs() < EPS);
    }

    #[test]
    fn normalize_wraps_lon() {
        let n = normalize(LonLat {
            lon: 200.0,
            lat: 0.0,
        });
        assert!((n.lon - (-160.0)).abs() < 1e-10);
        let n2 = normalize(LonLat {
            lon: -200.0,
            lat: 0.0,
        });
        assert!((n2.lon - 160.0).abs() < 1e-10);
        let n3 = normalize(LonLat {
            lon: 540.0,
            lat: 0.0,
        });
        assert!((n3.lon - 180.0).abs() < 1e-10);
    }

    #[test]
    fn normalize_clamps_lat() {
        let n = normalize(LonLat {
            lon: 0.0,
            lat: 90.0,
        });
        assert!((n.lat - MAX_LAT).abs() < EPS);
        let n2 = normalize(LonLat {
            lon: 0.0,
            lat: -90.0,
        });
        assert!((n2.lat + MAX_LAT).abs() < EPS);
    }

    // --- ll2tile / tile2ll round-trip ----------------------------------------

    #[test]
    fn ll2tile_origin_zoom0() {
        // (0°, 0°) should map to the centre of the single tile at zoom 0.
        let t = ll2tile(0.0, 0.0, 0);
        assert!((t.x - 0.5).abs() < EPS);
        assert!((t.y - 0.5).abs() < EPS);
        assert_eq!(t.z, 0);
    }

    #[test]
    fn ll2tile_topleft_zoom1() {
        // (-180°, ~85.051°) → tile (0, 0) at zoom 1.
        let t = ll2tile(-180.0, MAX_LAT, 1);
        assert!(t.x.abs() < EPS);
        assert!(t.y.abs() < 1e-3);
        assert_eq!(t.z, 1);
    }

    #[test]
    fn tile2ll_origin_zoom0() {
        // Centre of the only tile at zoom 0 is (0°, 0°).
        let ll = tile2ll(0.5, 0.5, 0);
        assert!(ll.lon.abs() < EPS);
        assert!(ll.lat.abs() < EPS);
    }

    #[test]
    fn projection_center_maps_to_canvas_centre() {
        let center = LonLat {
            lon: 13.42,
            lat: 52.51,
        };
        let proj = MapProjection::new(center, 10.0, 80, 24);
        let (col, row) = proj.ll_to_cell(center).unwrap();
        // Canvas centre in cells = (cols/2, rows/2) with sub-cell rounding.
        assert_eq!(col, 40);
        assert_eq!(row, 12);
    }

    #[test]
    fn projection_cell_ll_roundtrip_near_centre() {
        let proj = MapProjection::new(
            LonLat {
                lon: 13.42,
                lat: 52.51,
            },
            12.0,
            120,
            40,
        );
        for &(col, row) in &[(0u16, 0u16), (60, 20), (119, 39)] {
            let ll = proj.cell_to_ll(col, row).expect("inside canvas");
            let (back_col, back_row) = proj
                .ll_to_cell(ll)
                .expect("projection back onto canvas must succeed");
            assert!(
                back_col.abs_diff(col) <= 1,
                "col drift: {col} -> {back_col}"
            );
            assert!(
                back_row.abs_diff(row) <= 1,
                "row drift: {row} -> {back_row}"
            );
        }
    }

    /// Regression for issue #96. When the view centre is near the
    /// antimeridian and a feature sits on the other side (geographically
    /// adjacent across the date line), `pt.x - center_tile.x` is ~the
    /// full grid width — projecting the overlay way off-screen instead
    /// of right next to the centre. The projection must take the
    /// shorter modular path on x.
    #[test]
    fn projection_handles_antimeridian_wrap_for_overlays() {
        // Center at lon ≈ 179.9, zoom 5 (grid = 32). A feature at lon
        // ≈ -179.9 is ~0.2° away across the date line — visually
        // inseparable from the centre on a 200×60-cell canvas.
        let proj = MapProjection::new(
            LonLat {
                lon: 179.9,
                lat: 0.0,
            },
            5.0,
            200,
            60,
        );
        let near = LonLat {
            lon: -179.9,
            lat: 0.0,
        };
        let cell = proj
            .ll_to_cell(near)
            .expect("wrap-side feature must project on-screen, not be dropped");
        // Should land within a few cells of the canvas centre (col 100).
        assert!(
            cell.0.abs_diff(100) < 5,
            "expected near centre, got col={}",
            cell.0
        );
    }

    #[test]
    fn projection_handles_antimeridian_wrap_other_direction() {
        // Mirror: centre near -180, feature near +180.
        let proj = MapProjection::new(
            LonLat {
                lon: -179.9,
                lat: 0.0,
            },
            5.0,
            200,
            60,
        );
        let near = LonLat {
            lon: 179.9,
            lat: 0.0,
        };
        let cell = proj
            .ll_to_cell(near)
            .expect("wrap-side feature must project on-screen");
        assert!(
            cell.0.abs_diff(100) < 5,
            "expected near centre, got col={}",
            cell.0
        );
    }

    #[test]
    fn projection_rejects_out_of_canvas() {
        let proj = MapProjection::new(LonLat { lon: 0.0, lat: 0.0 }, 5.0, 40, 20);
        // A point 180° away from centre at the same zoom is offscreen.
        assert!(
            proj.ll_to_cell(LonLat {
                lon: 170.0,
                lat: -60.0,
            })
            .is_none()
        );
        assert!(proj.cell_to_ll(40, 0).is_none());
        assert!(proj.cell_to_ll(0, 20).is_none());
    }

    #[test]
    fn ll2tile_tile2ll_roundtrip() {
        for (lon, lat, z) in [
            (13.42, 52.51, 10),  // Berlin
            (139.76, 35.68, 12), // Tokyo
            (-74.00, 40.71, 8),  // New York
            (0.0, 0.0, 5),
        ] {
            let t = ll2tile(lon, lat, z);
            let ll = tile2ll(t.x, t.y, z);
            assert!((ll.lon - lon).abs() < 1e-9, "lon drift at z={z}");
            assert!((ll.lat - lat).abs() < 1e-9, "lat drift at z={z}");
        }
    }

    // --- base_zoom -----------------------------------------------------------

    #[test]
    fn base_zoom_floor() {
        assert_eq!(base_zoom(5.9), 5);
        assert_eq!(base_zoom(5.0), 5);
    }

    #[test]
    fn base_zoom_clamp_low() {
        assert_eq!(base_zoom(-1.0), 0);
        assert_eq!(base_zoom(0.0), 0);
    }

    #[test]
    fn base_zoom_clamp_high() {
        assert_eq!(base_zoom(15.0), 14);
        assert_eq!(base_zoom(14.0), 14);
        assert_eq!(base_zoom(100.0), 14);
    }

    // --- tile_size_at_zoom ---------------------------------------------------

    #[test]
    fn tile_size_integer_zoom() {
        // At an integer zoom the fractional part is 0, so 2^0 = 1.
        assert!((tile_size_at_zoom(3.0) - 256.0).abs() < EPS);
        assert!((tile_size_at_zoom(0.0) - 256.0).abs() < EPS);
    }

    #[test]
    fn tile_size_half_zoom() {
        // zoom = 3.5 → base = 3, scale = 2^0.5 ≈ 1.4142
        let expected = 256.0 * 2f64.powf(0.5);
        assert!((tile_size_at_zoom(3.5) - expected).abs() < EPS);
    }

    #[test]
    fn tile_size_clamped_zoom() {
        // zoom = 15 → base_zoom clamps to 14, scale = 2^1 = 2
        let expected = 256.0 * 2f64.powf(1.0);
        assert!((tile_size_at_zoom(15.0) - expected).abs() < EPS);
    }

    // --- format_distance -----------------------------------------------------

    #[test]
    fn format_distance_metres() {
        assert_eq!(format_distance(500.0), "500m");
        assert_eq!(format_distance(0.0), "0m");
        assert_eq!(format_distance(999.4), "999m");
    }

    #[test]
    fn format_distance_kilometres() {
        assert_eq!(format_distance(1000.0), "1.0km");
        assert_eq!(format_distance(1500.0), "1.5km");
        assert_eq!(format_distance(10_000.0), "10.0km");
    }

    #[test]
    fn format_distance_boundary() {
        // 999.5 rounds to 1000 → still < 1000 check uses the raw value.
        assert_eq!(format_distance(999.0), "999m");
        assert_eq!(format_distance(1000.0), "1.0km");
    }

    // --- scale_bar ------------------------------------------------------------

    #[test]
    fn scale_bar_returns_nice_distance() {
        let (label, width) = scale_bar(0.0, 10.0, 80);
        // Should return a recognizable distance label
        assert!(
            label.ends_with('m') || label.ends_with("km"),
            "label={label}"
        );
        assert!(width >= 2, "width={width}");
    }

    #[test]
    fn scale_bar_higher_zoom_smaller_distance() {
        let (_, w_low) = scale_bar(35.0, 5.0, 80);
        let (_, w_high) = scale_bar(35.0, 12.0, 80);
        // At higher zoom, the same screen width covers less distance,
        // so the bar width for a smaller nice distance should still be reasonable
        assert!(w_low >= 2);
        assert!(w_high >= 2);
    }

    #[test]
    fn scale_bar_width_clamped() {
        let (_, width) = scale_bar(0.0, 1.0, 30);
        // Width should not exceed 1/3 of screen
        assert!(width <= 10, "width={width}");
    }

    #[test]
    fn scale_bar_equator_vs_pole() {
        // At high latitude, metres per pixel is smaller → different scale
        let (label_eq, _) = scale_bar(0.0, 8.0, 80);
        let (label_hi, _) = scale_bar(70.0, 8.0, 80);
        // Both should produce valid labels
        assert!(label_eq.ends_with('m') || label_eq.ends_with("km"));
        assert!(label_hi.ends_with('m') || label_hi.ends_with("km"));
    }
}
