//! Tile-coord → screen-pixel projection helpers.
//!
//! One pure function (no `Renderer` state), [`scale_ring_into`], that
//! the line and fill draw paths share. Lifted out of `renderer.rs` so
//! the projection math can be unit-tested without standing up a
//! [`Renderer`] (and so the test names sit next to the implementation
//! they exercise).
//!
//! Coordinate pipeline for each point: `TilePoint` (i32, 0..extent)
//! → f64 screen offset relative to `vis.pos_{x,y}` → final screen
//! pixel (i32). The f64 step is where we spend precision for the
//! divide; the final `as i32` rounds toward zero, matching the
//! tolerance of the braille pixel grid. `inv_scale = 1.0 / scale`
//! is precomputed so each point is a multiply rather than a divide.

use super::view::VisibleTile;
use crate::core::map::tile::decode::TilePoint;

/// Project tile-local points into integer screen pixels and append
/// them to `out`, optionally clipping against the padded viewport.
///
/// Behaviour notes (regression: issue #103):
/// - Consecutive duplicate output points are collapsed.
/// - When `clip` is true, runs of consecutive outside points are
///   collapsed too — but on **re-entry** (outside → inside) the
///   *last* outside point must be emitted before the inside point so
///   the polyline draws the actual off-screen kink, not a chord
///   straight across the viewport.
pub fn scale_ring_into(
    out: &mut Vec<(i32, i32)>,
    vis: &VisibleTile,
    ring: &[TilePoint],
    scale: f64,
    width: usize,
    height: usize,
    clip: bool,
) {
    out.clear();
    let pad = super::VIEWPORT_PADDING;
    let min_x = -pad;
    let min_y = -pad;
    let max_x = width as i32 + pad;
    let max_y = height as i32 + pad;
    let inv_scale = 1.0 / scale;

    let mut last = (i32::MIN, i32::MIN);
    let mut outside = false;

    for p in ring {
        let pt = (
            (vis.pos_x + p.x as f64 * inv_scale) as i32,
            (vis.pos_y + p.y as f64 * inv_scale) as i32,
        );
        if pt == last {
            continue;
        }
        // Capture the previous processed point *before* advancing
        // `last`, so the re-entry branch below can emit the actual
        // last-outside point (the kink the polyline turns at)
        // rather than the current inside point.
        let prev = last;
        last = pt;

        if clip {
            let is_out = pt.0 < min_x || pt.0 > max_x || pt.1 < min_y || pt.1 > max_y;
            if is_out {
                if outside {
                    continue;
                }
                outside = true;
            } else if outside {
                outside = false;
                out.push(prev);
            }
        }
        out.push(pt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for issue #103. `scale_ring_into` collapses runs of
    /// consecutive outside points into a single emitted point, but on
    /// re-entry it must emit the *last* outside point (so the renderer
    /// draws the segment from there to the inside re-entry point). The
    /// pre-fix code reassigned `last` before the clip check and ended
    /// up pushing the *current* (inside) point twice — producing a
    /// straight chord across the viewport instead of following the
    /// off-screen geometry.
    #[test]
    fn scale_ring_into_preserves_last_outside_point_on_reentry() {
        // Canvas 100×100 px with VIEWPORT_PADDING=64 → clip box
        // [-64, 164] × [-64, 164]. scale=1.0 → tile coords map 1:1
        // to screen pixels.
        let vis = VisibleTile {
            x: 0,
            y: 0,
            z: 0,
            pos_x: 0.0,
            pos_y: 0.0,
            size: 256.0,
        };
        let ring = vec![
            TilePoint { x: 50, y: 50 },  // inside
            TilePoint { x: 200, y: 50 }, // outside (x > 164)
            TilePoint { x: 250, y: 50 }, // outside
            TilePoint { x: 50, y: 80 },  // inside (re-entry)
        ];
        let mut out: Vec<(i32, i32)> = Vec::new();
        scale_ring_into(&mut out, &vis, &ring, 1.0, 100, 100, true);

        // Expected: emit first-inside, first-outside (so the in→out
        // segment renders), last-outside (so the out→in segment
        // renders correctly), and the re-entry inside point. No
        // duplicates.
        assert_eq!(
            out,
            vec![(50, 50), (200, 50), (250, 50), (50, 80)],
            "re-entry must preserve the last outside point as the kink \
             before resuming inside, not duplicate the inside point"
        );
    }

    /// All-inside path is unaffected: every point is emitted in order,
    /// with consecutive duplicates collapsed.
    #[test]
    fn scale_ring_into_all_inside_emits_every_point() {
        let vis = VisibleTile {
            x: 0,
            y: 0,
            z: 0,
            pos_x: 0.0,
            pos_y: 0.0,
            size: 256.0,
        };
        let ring = vec![
            TilePoint { x: 10, y: 10 },
            TilePoint { x: 20, y: 20 },
            TilePoint { x: 30, y: 30 },
        ];
        let mut out: Vec<(i32, i32)> = Vec::new();
        scale_ring_into(&mut out, &vis, &ring, 1.0, 100, 100, true);
        assert_eq!(out, vec![(10, 10), (20, 20), (30, 30)]);
    }
}
