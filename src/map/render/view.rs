//! Map view — determines which tiles are visible at the current center/zoom.
//! Pure calculation, no side effects.

use crate::geo;

/// A tile positioned on screen (pixel coordinates).
#[derive(Clone)]
pub struct VisibleTile {
    pub x: i32,
    pub y: i32,
    pub z: u32,
    pub pos_x: f64,
    pub pos_y: f64,
    pub size: f64,
}

/// Calculate which tiles are visible for the given center, zoom, and screen size.
pub fn visible_tiles(
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    width: usize,
    height: usize,
) -> Vec<VisibleTile> {
    let z = geo::base_zoom(zoom);
    let center = geo::ll2tile(center_lon, center_lat, z);
    let tile_size = geo::tile_size_at_zoom(zoom);
    let grid_size = (1u64 << z) as i32;

    let mut tiles = Vec::new();

    // Dynamic scan radius: cover the viewport plus one tile of slack
    // on each side so we never leave a gap at the edges (issue #95).
    // The per-tile visibility check below filters anything that falls
    // off-screen, so being generous here is cheap.
    let half_w_tiles = (width as f64 / 2.0 / tile_size).ceil() as i32 + 1;
    let half_h_tiles = (height as f64 / 2.0 / tile_size).ceil() as i32 + 1;

    for dy in -half_h_tiles..=half_h_tiles {
        for dx in -half_w_tiles..=half_w_tiles {
            let tile_x = center.x.floor() as i32 + dx;
            let tile_y = center.y.floor() as i32 + dy;

            if tile_y < 0 || tile_y >= grid_size {
                continue;
            }

            let pos_x = width as f64 / 2.0 - (center.x - tile_x as f64) * tile_size;
            let pos_y = height as f64 / 2.0 - (center.y - tile_y as f64) * tile_size;

            if pos_x >= width as f64
                || pos_y >= height as f64
                || pos_x + tile_size <= 0.0
                || pos_y + tile_size <= 0.0
            {
                continue;
            }

            let wrapped_x = tile_x.rem_euclid(grid_size);

            tiles.push(VisibleTile {
                x: wrapped_x,
                y: tile_y,
                z,
                pos_x,
                pos_y,
                size: tile_size,
            });
        }
    }

    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_tiles_at_zoom0() {
        // Zoom 0: only 1 tile in the grid (0,0)
        let tiles = visible_tiles(0.0, 0.0, 0.0, 200, 200);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.z == 0));
    }

    #[test]
    fn test_visible_tiles_at_zoom1() {
        // Zoom 1: 2x2 grid, Berlin should see a few tiles
        let tiles = visible_tiles(13.4, 52.5, 1.0, 400, 400);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.z == 1));
    }

    #[test]
    fn test_visible_tiles_wraps_longitude() {
        // Near the date line — tile x should wrap
        let tiles = visible_tiles(179.9, 0.0, 2.0, 400, 400);
        assert!(!tiles.is_empty());
        for t in &tiles {
            assert!(t.x >= 0, "tile x should be non-negative after wrapping");
        }
    }

    #[test]
    fn test_visible_tiles_polar_clamp() {
        // Near north pole — some y tiles should be skipped
        let tiles = visible_tiles(0.0, 85.0, 2.0, 400, 400);
        for t in &tiles {
            let grid = (1u64 << t.z) as i32;
            assert!(
                t.y >= 0 && t.y < grid,
                "tile y={} out of grid {}",
                t.y,
                grid
            );
        }
    }

    /// Regression for issue #95. A 3×3 fixed scan covers at most
    /// `3 × tile_size` pixels in each direction (~768 at z≥1). On
    /// large terminals, the viewport edges are left blank — visible
    /// black gaps. The visible tile set must scale with screen size.
    #[test]
    fn visible_tiles_cover_large_viewport() {
        // 2000×1000 px viewport at z=4 → tile_size=256. The viewport
        // is ~7.8 tiles wide and ~3.9 tall; a 3×3 scan reaches only
        // 768 px on each axis so the right/bottom would be uncovered.
        let tiles = visible_tiles(0.0, 0.0, 4.0, 2000, 1000);
        assert!(!tiles.is_empty());

        let min_x = tiles.iter().map(|t| t.pos_x).fold(f64::INFINITY, f64::min);
        let max_x = tiles
            .iter()
            .map(|t| t.pos_x + t.size)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = tiles.iter().map(|t| t.pos_y).fold(f64::INFINITY, f64::min);
        let max_y = tiles
            .iter()
            .map(|t| t.pos_y + t.size)
            .fold(f64::NEG_INFINITY, f64::max);

        assert!(min_x <= 0.0, "left edge uncovered (min_x={min_x})");
        assert!(max_x >= 2000.0, "right edge uncovered (max_x={max_x})");
        assert!(min_y <= 0.0, "top edge uncovered (min_y={min_y})");
        assert!(max_y >= 1000.0, "bottom edge uncovered (max_y={max_y})");
    }

    #[test]
    fn test_visible_tiles_size_positive() {
        let tiles = visible_tiles(0.0, 0.0, 3.0, 300, 300);
        for t in &tiles {
            assert!(t.size > 0.0);
        }
    }
}
