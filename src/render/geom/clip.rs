//! Line and polygon clipping against an axis-aligned rectangle.
//!
//! - [`clip_line`] — Cohen-Sutherland for line segments.
//! - [`sutherland_hodgman_into`] — Sutherland-Hodgman for polygon rings;
//!   the `*_into` variant lets the caller supply reusable scratch and
//!   output buffers so no allocations happen on the hot path.

// ── Cohen-Sutherland outcode bits ────────────────────────────────────────────

const INSIDE: u8 = 0b0000;
const LEFT: u8 = 0b0001;
const RIGHT: u8 = 0b0010;
const BOTTOM: u8 = 0b0100;
const TOP: u8 = 0b1000;

fn outcode(x: i32, y: i32, xmin: i32, ymin: i32, xmax: i32, ymax: i32) -> u8 {
    let mut code = INSIDE;
    if x < xmin {
        code |= LEFT;
    } else if x > xmax {
        code |= RIGHT;
    }
    if y < ymin {
        code |= TOP;
    } else if y > ymax {
        code |= BOTTOM;
    }
    code
}

/// Clip a line segment to the rectangle `bounds = (xmin, ymin, xmax, ymax)`
/// using Cohen-Sutherland. Returns `None` if the segment is entirely outside.
pub(crate) fn clip_line(
    bounds: (i32, i32, i32, i32),
    mut x0: i32,
    mut y0: i32,
    mut x1: i32,
    mut y1: i32,
) -> Option<(i32, i32, i32, i32)> {
    let (xmin, ymin, xmax, ymax) = bounds;
    let mut code0 = outcode(x0, y0, xmin, ymin, xmax, ymax);
    let mut code1 = outcode(x1, y1, xmin, ymin, xmax, ymax);

    for _ in 0..20 {
        if (code0 | code1) == 0 {
            return Some((x0, y0, x1, y1));
        }
        if (code0 & code1) != 0 {
            return None;
        }

        let code_out = if code0 != 0 { code0 } else { code1 };
        let (x, y);

        if code_out & BOTTOM != 0 {
            x = x0 + ((x1 - x0) as i64 * (ymax - y0) as i64 / (y1 - y0) as i64) as i32;
            y = ymax;
        } else if code_out & TOP != 0 {
            x = x0 + ((x1 - x0) as i64 * (ymin - y0) as i64 / (y1 - y0) as i64) as i32;
            y = ymin;
        } else if code_out & RIGHT != 0 {
            y = y0 + ((y1 - y0) as i64 * (xmax - x0) as i64 / (x1 - x0) as i64) as i32;
            x = xmax;
        } else {
            y = y0 + ((y1 - y0) as i64 * (xmin - x0) as i64 / (x1 - x0) as i64) as i32;
            x = xmin;
        }

        if code_out == code0 {
            x0 = x;
            y0 = y;
            code0 = outcode(x0, y0, xmin, ymin, xmax, ymax);
        } else {
            x1 = x;
            y1 = y;
            code1 = outcode(x1, y1, xmin, ymin, xmax, ymax);
        }
    }

    None
}

// ── Sutherland-Hodgman polygon clipping ──────────────────────────────────────

/// Clip a polygon ring to the rectangle `bounds = (xmin, ymin, xmax, ymax)`.
/// Writes the clipped ring into `output`. `buf_a` and `buf_b` are scratch
/// ping-pong buffers used between the 4 edge-clipping stages; their contents
/// are overwritten.
pub(crate) fn sutherland_hodgman_into(
    polygon: &[(i32, i32)],
    bounds: (i32, i32, i32, i32),
    buf_a: &mut Vec<(i32, i32)>,
    buf_b: &mut Vec<(i32, i32)>,
    output: &mut Vec<(i32, i32)>,
) {
    output.clear();
    if polygon.is_empty() {
        return;
    }

    let (xmin, ymin, xmax, ymax) = bounds;

    // Stage the polygon in buf_a, then pipe through 4 edges. The final edge
    // (bottom) writes directly into `output`.
    buf_a.clear();
    buf_a.extend_from_slice(polygon);

    buf_b.clear();
    clip_against_edge(buf_a, buf_b, true, true, xmin); // left
    if buf_b.is_empty() {
        return;
    }

    buf_a.clear();
    clip_against_edge(buf_b, buf_a, true, false, xmax); // right
    if buf_a.is_empty() {
        return;
    }

    buf_b.clear();
    clip_against_edge(buf_a, buf_b, false, true, ymin); // top
    if buf_b.is_empty() {
        return;
    }

    clip_against_edge(buf_b, output, false, false, ymax); // bottom
}

/// Clip `input` against a single axis-aligned edge and append the result to
/// `output`. `axis == true` selects the x coordinate; `keep_ge == true` keeps
/// points with coord >= `bound`, otherwise <= `bound`.
fn clip_against_edge(
    input: &[(i32, i32)],
    output: &mut Vec<(i32, i32)>,
    axis: bool,
    keep_ge: bool,
    bound: i32,
) {
    let val = |p: (i32, i32)| if axis { p.0 } else { p.1 };
    let inside = |p: (i32, i32)| {
        if keep_ge {
            val(p) >= bound
        } else {
            val(p) <= bound
        }
    };
    let intersect = |a: (i32, i32), b: (i32, i32)| {
        if axis {
            intersect_x(a, b, bound)
        } else {
            intersect_y(a, b, bound)
        }
    };

    let mut prev = input[input.len() - 1];
    let mut prev_inside = inside(prev);

    for &curr in input {
        let curr_inside = inside(curr);
        if curr_inside {
            if !prev_inside {
                output.push(intersect(prev, curr));
            }
            output.push(curr);
        } else if prev_inside {
            output.push(intersect(prev, curr));
        }
        prev = curr;
        prev_inside = curr_inside;
    }
}

fn intersect_x(a: (i32, i32), b: (i32, i32), x: i32) -> (i32, i32) {
    let dx = b.0 as i64 - a.0 as i64;
    if dx == 0 {
        return (x, a.1);
    }
    let t = (x as i64 - a.0 as i64) as f64 / dx as f64;
    let y = a.1 as f64 + t * (b.1 as f64 - a.1 as f64);
    (x, y as i32)
}

fn intersect_y(a: (i32, i32), b: (i32, i32), y: i32) -> (i32, i32) {
    let dy = b.1 as i64 - a.1 as i64;
    if dy == 0 {
        return (a.0, y);
    }
    let t = (y as i64 - a.1 as i64) as f64 / dy as f64;
    let x = a.0 as f64 + t * (b.0 as f64 - a.0 as f64);
    (x as i32, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::VIEWPORT_PADDING;

    /// The padded-viewport bounds that `Canvas` of size `w×h` computes
    /// internally — replicated here so tests can target the same shape.
    fn canvas_bounds(w: i32, h: i32) -> (i32, i32, i32, i32) {
        (
            -VIEWPORT_PADDING,
            -VIEWPORT_PADDING,
            w + VIEWPORT_PADDING,
            h + VIEWPORT_PADDING,
        )
    }

    /// Owned-result variant of `sutherland_hodgman_into` used by tests.
    fn sutherland_hodgman(
        polygon: &[(i32, i32)],
        xmin: i32,
        ymin: i32,
        xmax: i32,
        ymax: i32,
    ) -> Vec<(i32, i32)> {
        let mut buf_a = Vec::new();
        let mut buf_b = Vec::new();
        let mut output = Vec::new();
        sutherland_hodgman_into(
            polygon,
            (xmin, ymin, xmax, ymax),
            &mut buf_a,
            &mut buf_b,
            &mut output,
        );
        output
    }

    // ── clip_line ─────────────────────────────────────────────────────────

    #[test]
    fn test_clip_line_fully_inside() {
        assert!(clip_line(canvas_bounds(100, 100), 10, 10, 50, 50).is_some());
    }

    #[test]
    fn test_clip_line_fully_outside() {
        // Both points far to the right
        assert!(clip_line(canvas_bounds(100, 100), 10000, 0, 20000, 50).is_none());
    }

    #[test]
    fn test_clip_line_crosses_viewport() {
        let result = clip_line(canvas_bounds(100, 100), -1000, 50, 1000, 50);
        assert!(result.is_some());
        let (cx0, _, cx1, _) = result.unwrap();
        assert!(cx0 >= -VIEWPORT_PADDING);
        assert!(cx1 <= 100 + VIEWPORT_PADDING);
    }

    // ── Sutherland-Hodgman ───────────────────────────────────────────────

    #[test]
    fn test_sh_polygon_fully_inside() {
        let poly = vec![(10, 10), (50, 10), (50, 50), (10, 50)];
        let clipped = sutherland_hodgman(&poly, 0, 0, 100, 100);
        assert_eq!(clipped.len(), 4);
    }

    #[test]
    fn test_sh_polygon_fully_outside() {
        let poly = vec![(200, 200), (300, 200), (300, 300), (200, 300)];
        let clipped = sutherland_hodgman(&poly, 0, 0, 100, 100);
        assert!(clipped.is_empty());
    }

    #[test]
    fn test_sh_polygon_partially_inside() {
        // Square from (-50,-50) to (50,50), clipped to (0,0)-(100,100)
        let poly = vec![(-50, -50), (50, -50), (50, 50), (-50, 50)];
        let clipped = sutherland_hodgman(&poly, 0, 0, 100, 100);
        // Should produce a polygon covering (0,0)-(50,50) area
        assert!(clipped.len() >= 3);
        for &(x, y) in &clipped {
            assert!(x >= 0 && x <= 100, "x={} out of bounds", x);
            assert!(y >= 0 && y <= 100, "y={} out of bounds", y);
        }
    }

    #[test]
    fn test_sh_large_polygon_covering_viewport() {
        // Huge square that fully covers the viewport
        let poly = vec![(-1000, -1000), (2000, -1000), (2000, 2000), (-1000, 2000)];
        let clipped = sutherland_hodgman(&poly, 0, 0, 100, 100);
        // Should be clipped to the viewport rectangle
        assert_eq!(clipped.len(), 4);
        assert!(clipped.contains(&(0, 0)));
        assert!(clipped.contains(&(100, 0)));
        assert!(clipped.contains(&(100, 100)));
        assert!(clipped.contains(&(0, 100)));
    }
}
