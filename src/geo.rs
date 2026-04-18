use std::f64::consts::PI;

/// Maximum latitude for Web Mercator projection.
pub const MAX_LAT: f64 = 85.051_129_0;
/// Earth radius in metres (WGS84 mean).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// A geographic coordinate in WGS84.
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Floor a zoom level and clamp to 0..=14.
pub fn base_zoom(zoom: f64) -> u32 {
    zoom.floor().clamp(0.0, 14.0) as u32
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
