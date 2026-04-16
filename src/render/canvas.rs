use super::braille::BrailleBuffer;
use super::frame::MapFrame;
use super::label::LabelBuffer;

use super::VIEWPORT_PADDING;

pub struct Canvas {
    width: usize,
    height: usize,
    buffer: BrailleBuffer,
    labels: LabelBuffer,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            height,
            buffer: BrailleBuffer::new(width, height),
            labels: LabelBuffer::new(),
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.labels.clear();
    }

    pub fn set_background(&mut self, color: u8) {
        self.buffer.set_global_background(color);
    }

    pub fn to_map_frame(&self) -> MapFrame {
        self.buffer.to_map_frame()
    }

    pub fn text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        self.buffer.write_text(text, x, y, color);
    }

    /// Try to place a label at the given position. Returns true if placed
    /// (no collision with existing labels).
    pub fn try_place_label(&mut self, text: &str, x: f64, y: f64) -> bool {
        self.labels.write_if_possible(text, x, y, None)
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u8) {
        self.buffer.set_pixel(x, y, color);
    }

    pub fn polyline(&mut self, points: &[(i32, i32)], color: u8) {
        if points.len() < 2 {
            return;
        }
        for i in 0..points.len() - 1 {
            let (x0, y0) = points[i];
            let (x1, y1) = points[i + 1];
            self.draw_line_clipped(x0, y0, x1, y1, color);
        }
    }

    pub fn polygon(&mut self, rings: &[Vec<(i32, i32)>], color: u8) {
        if rings.is_empty() || rings[0].len() < 3 {
            return;
        }

        // Skip degenerate: tiny outer ring with holes
        if rings.len() > 1 && rings[0].len() < 4 {
            return;
        }

        // Collect vertices as [f64; 2] and hole indices
        let mut vertices: Vec<[f64; 2]> = Vec::new();
        let mut hole_indices: Vec<usize> = Vec::new();

        for &(x, y) in &rings[0] {
            vertices.push([x as f64, y as f64]);
        }

        for ring in &rings[1..] {
            if ring.len() < 3 {
                continue;
            }
            hole_indices.push(vertices.len());
            for &(x, y) in ring {
                vertices.push([x as f64, y as f64]);
            }
        }

        // Triangulate using earcut (ciscorn/earcut-rs, based on earcut 3.0.1)
        // catch_unwind: earcut can panic on degenerate geometry (e.g. unwrap on None)
        // Suppress panic hook output to avoid noise on stderr.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(|| {
            let mut earcut = earcut::Earcut::new();
            let mut indices: Vec<usize> = Vec::new();
            earcut.earcut(vertices.iter().copied(), &hole_indices, &mut indices);
            indices
        });
        std::panic::set_hook(prev_hook);
        let indices = match result {
            Ok(idx) => idx,
            Err(_) => return, // skip polygon on earcut panic
        };

        if indices.is_empty() {
            return;
        }

        let mut i = 0;
        while i + 2 < indices.len() {
            let ia = indices[i];
            let ib = indices[i + 1];
            let ic = indices[i + 2];
            let a = [vertices[ia][0] as i32, vertices[ia][1] as i32];
            let b = [vertices[ib][0] as i32, vertices[ib][1] as i32];
            let c = [vertices[ic][0] as i32, vertices[ic][1] as i32];
            self.filled_triangle(a, b, c, color);
            i += 3;
        }
    }

    // ── Sutherland-Hodgman polygon clipping ─────────────────────────────────

    /// Clip a polygon to the padded viewport using Sutherland-Hodgman algorithm.
    /// Each ring is clipped independently. Returns clipped rings (may be empty
    /// if entirely outside).
    pub(super) fn clip_polygon(&self, rings: &[Vec<(i32, i32)>]) -> Vec<Vec<(i32, i32)>> {
        let (xmin, ymin, xmax, ymax) = self.clip_bounds();
        rings
            .iter()
            .filter_map(|ring| {
                let clipped = sutherland_hodgman(ring, xmin, ymin, xmax, ymax);
                if clipped.len() >= 3 {
                    Some(clipped)
                } else {
                    None
                }
            })
            .collect()
    }

    // ── Cohen-Sutherland line clipping ────────────────────────────────────────

    fn clip_bounds(&self) -> (i32, i32, i32, i32) {
        (
            -VIEWPORT_PADDING,
            -VIEWPORT_PADDING,
            self.width as i32 + VIEWPORT_PADDING,
            self.height as i32 + VIEWPORT_PADDING,
        )
    }

    const INSIDE: u8 = 0b0000;
    const LEFT: u8 = 0b0001;
    const RIGHT: u8 = 0b0010;
    const BOTTOM: u8 = 0b0100;
    const TOP: u8 = 0b1000;

    fn outcode(&self, x: i32, y: i32, xmin: i32, ymin: i32, xmax: i32, ymax: i32) -> u8 {
        let mut code = Self::INSIDE;
        if x < xmin {
            code |= Self::LEFT;
        } else if x > xmax {
            code |= Self::RIGHT;
        }
        if y < ymin {
            code |= Self::TOP;
        } else if y > ymax {
            code |= Self::BOTTOM;
        }
        code
    }

    /// Clip a line segment to the padded viewport. Returns None if entirely outside.
    fn clip_line(
        &self,
        mut x0: i32,
        mut y0: i32,
        mut x1: i32,
        mut y1: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let (xmin, ymin, xmax, ymax) = self.clip_bounds();
        let mut code0 = self.outcode(x0, y0, xmin, ymin, xmax, ymax);
        let mut code1 = self.outcode(x1, y1, xmin, ymin, xmax, ymax);

        for _ in 0..20 {
            if (code0 | code1) == 0 {
                // Both inside
                return Some((x0, y0, x1, y1));
            }
            if (code0 & code1) != 0 {
                // Both on same outside side
                return None;
            }

            let code_out = if code0 != 0 { code0 } else { code1 };
            let (x, y);

            if code_out & Self::BOTTOM != 0 {
                x = x0 + ((x1 - x0) as i64 * (ymax - y0) as i64 / (y1 - y0) as i64) as i32;
                y = ymax;
            } else if code_out & Self::TOP != 0 {
                x = x0 + ((x1 - x0) as i64 * (ymin - y0) as i64 / (y1 - y0) as i64) as i32;
                y = ymin;
            } else if code_out & Self::RIGHT != 0 {
                y = y0 + ((y1 - y0) as i64 * (xmax - x0) as i64 / (x1 - x0) as i64) as i32;
                x = xmax;
            } else {
                y = y0 + ((y1 - y0) as i64 * (xmin - x0) as i64 / (x1 - x0) as i64) as i32;
                x = xmin;
            }

            if code_out == code0 {
                x0 = x;
                y0 = y;
                code0 = self.outcode(x0, y0, xmin, ymin, xmax, ymax);
            } else {
                x1 = x;
                y1 = y;
                code1 = self.outcode(x1, y1, xmin, ymin, xmax, ymax);
            }
        }

        None
    }

    // ── Drawing with clipping ─────────────────────────────────────────────────

    fn draw_line_clipped(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        if let Some((cx0, cy0, cx1, cy1)) = self.clip_line(x0, y0, x1, y1) {
            self.line_bresenham(cx0, cy0, cx1, cy1, color);
        }
    }

    fn line_bresenham(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        for (x, y) in BresenhamIter::new(x0, y0, x1, y1) {
            if x >= 0 && y >= 0 {
                self.buffer.set_pixel(x as usize, y as usize, color);
            }
        }
    }

    fn filled_triangle(&mut self, a: [i32; 2], b: [i32; 2], c: [i32; 2], color: u8) {
        let mut edge_pixels: Vec<(i32, i32)> = Vec::new();

        for pair in [(&a, &b), (&b, &c), (&c, &a)] {
            if let Some((cx0, cy0, cx1, cy1)) =
                self.clip_line(pair.0[0], pair.0[1], pair.1[0], pair.1[1])
            {
                edge_pixels.extend(BresenhamIter::new(cx0, cy0, cx1, cy1));
            }
        }

        if edge_pixels.is_empty() {
            return;
        }

        let h = self.height as i32;
        let w = self.width as i32;
        edge_pixels.retain(|p| p.1 >= 0 && p.1 < h);
        edge_pixels.sort_by(|p1, p2| p1.1.cmp(&p2.1).then(p1.0.cmp(&p2.0)));

        let mut i = 0;
        while i < edge_pixels.len() {
            let y = edge_pixels[i].1;
            let start = i;
            while i < edge_pixels.len() && edge_pixels[i].1 == y {
                i += 1;
            }
            let row = &edge_pixels[start..i];
            let min_x = row.iter().map(|p| p.0).min().unwrap().max(0);
            let max_x = row.iter().map(|p| p.0).max().unwrap().min(w - 1);
            for x in min_x..=max_x {
                self.buffer.set_pixel(x as usize, y as usize, color);
            }
        }
    }
}

// ── Sutherland-Hodgman polygon clipping ───────────────────────────────────────

/// Clip a polygon ring to a rectangle using the Sutherland-Hodgman algorithm.
/// Clips against each of the 4 edges (left, right, bottom, top) in sequence.
fn sutherland_hodgman(
    polygon: &[(i32, i32)],
    xmin: i32,
    ymin: i32,
    xmax: i32,
    ymax: i32,
) -> Vec<(i32, i32)> {
    if polygon.is_empty() {
        return Vec::new();
    }

    let mut output = polygon.to_vec();

    // Clip against each of the 4 rectangle edges
    for &(axis, keep_ge, bound) in &[
        (true, true, xmin),   // left:   keep x >= xmin
        (true, false, xmax),  // right:  keep x <= xmax
        (false, true, ymin),  // top:    keep y >= ymin
        (false, false, ymax), // bottom: keep y <= ymax
    ] {
        if output.is_empty() {
            break;
        }
        let input = output;
        output = Vec::with_capacity(input.len());

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

        for &curr in &input {
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

    output
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

// ── Bresenham line iterator ───────────────────────────────────────────────────

struct BresenhamIter {
    x: i32,
    y: i32,
    x1: i32,
    y1: i32,
    dx: i32,
    dy: i32,
    sx: i32,
    sy: i32,
    err: i32,
    done: bool,
}

impl BresenhamIter {
    fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let dx = (x1 - x0).abs();
        let dy = (y1 - y0).abs();
        Self {
            x: x0,
            y: y0,
            x1,
            y1,
            dx,
            dy,
            sx: if x0 < x1 { 1 } else { -1 },
            sy: if y0 < y1 { 1 } else { -1 },
            err: dx - dy,
            done: false,
        }
    }
}

impl Iterator for BresenhamIter {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<(i32, i32)> {
        if self.done {
            return None;
        }
        let point = (self.x, self.y);
        if self.x == self.x1 && self.y == self.y1 {
            self.done = true;
            return Some(point);
        }
        let e2 = 2 * self.err;
        if e2 > -self.dy {
            self.err -= self.dy;
            self.x += self.sx;
        }
        if e2 < self.dx {
            self.err += self.dx;
            self.y += self.sy;
        }
        Some(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_canvas() {
        let canvas = Canvas::new(80, 40);
        assert_eq!(canvas.width, 80);
        assert_eq!(canvas.height, 40);
    }

    #[test]
    fn test_clear_and_frame() {
        let mut canvas = Canvas::new(80, 40);
        canvas.set_pixel(0, 0, 7);
        canvas.clear();
        let frame = canvas.to_map_frame();
        assert!(!frame.cells.is_empty());
    }

    #[test]
    fn test_polyline_draws_pixels() {
        let mut canvas = Canvas::new(80, 40);
        canvas.polyline(&[(0, 0), (10, 10)], 7);
        let frame = canvas.to_map_frame();
        let has_braille = frame
            .cells
            .iter()
            .any(|c| c.ch > '\u{2800}' && c.ch <= '\u{28FF}');
        assert!(
            has_braille,
            "polyline should draw pixels visible as braille chars"
        );
    }

    #[test]
    fn test_polygon_draws_pixels() {
        let mut canvas = Canvas::new(80, 40);
        let ring = vec![(5, 5), (20, 5), (12, 20)];
        canvas.polygon(&[ring], 7);
        let frame = canvas.to_map_frame();
        let has_braille = frame
            .cells
            .iter()
            .any(|c| c.ch > '\u{2800}' && c.ch <= '\u{28FF}');
        assert!(
            has_braille,
            "polygon should draw pixels visible as braille chars"
        );
    }

    #[test]
    fn test_text_appears_in_frame() {
        let mut canvas = Canvas::new(80, 40);
        canvas.text("Hi", 0, 0, 7);
        let frame = canvas.to_map_frame();
        assert!(
            frame.cells.iter().any(|c| c.ch == 'H'),
            "frame should contain 'H'"
        );
    }

    #[test]
    fn test_clip_line_fully_inside() {
        let canvas = Canvas::new(100, 100);
        assert!(canvas.clip_line(10, 10, 50, 50).is_some());
    }

    #[test]
    fn test_clip_line_fully_outside() {
        let canvas = Canvas::new(100, 100);
        // Both points far to the right
        assert!(canvas.clip_line(10000, 0, 20000, 50).is_none());
    }

    #[test]
    fn test_clip_line_crosses_viewport() {
        let canvas = Canvas::new(100, 100);
        let result = canvas.clip_line(-1000, 50, 1000, 50);
        assert!(result.is_some());
        let (cx0, _, cx1, _) = result.unwrap();
        // Should be clipped to viewport + padding
        assert!(cx0 >= -VIEWPORT_PADDING);
        assert!(cx1 <= 100 + VIEWPORT_PADDING);
    }

    #[test]
    fn test_huge_offscreen_line_no_hang() {
        let mut canvas = Canvas::new(100, 100);
        canvas.polyline(&[(50, 50), (1000000, 1000000)], 7);
    }

    // ── Sutherland-Hodgman tests ─────────────────────────────────────────

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

    #[test]
    fn test_clip_polygon_method() {
        let canvas = Canvas::new(100, 100);
        // Polygon that extends beyond viewport + padding
        let ring = vec![(-200, -200), (200, -200), (200, 200), (-200, 200)];
        let clipped = canvas.clip_polygon(&[ring]);
        assert_eq!(clipped.len(), 1);
        assert!(clipped[0].len() >= 3);
    }

    #[test]
    fn test_large_polygon_no_hang() {
        let mut canvas = Canvas::new(200, 200);
        // 10000-vertex polygon far off-screen — previously caused earcut hang
        let ring: Vec<(i32, i32)> = (0..10000)
            .map(|i| {
                let angle = (i as f64) * std::f64::consts::TAU / 10000.0;
                let r = 50000.0;
                ((r * angle.cos()) as i32, (r * angle.sin()) as i32)
            })
            .collect();
        // Clip first, then polygon — should be fast
        let clipped = canvas.clip_polygon(&[ring]);
        for r in &clipped {
            canvas.polygon(&[r.clone()], 7);
        }
        // Should complete without hanging
    }

    // ── Degenerate polygon tests ─────────────────────────────────────────

    #[test]
    fn test_polygon_collinear_no_hang() {
        let mut canvas = Canvas::new(100, 100);
        let ring = vec![(10, 10), (50, 10), (90, 10)];
        canvas.polygon(&[ring], 7);
    }

    #[test]
    fn test_polygon_degenerate_outer_with_holes_skipped() {
        let mut canvas = Canvas::new(100, 100);
        // Outer ring with only 3 vertices + a hole — degenerate
        let outer = vec![(10, 10), (50, 10), (30, 30)];
        let hole = vec![(20, 15), (30, 15), (25, 20)];
        // Should not hang (outer_len < 4 with holes → skip)
        canvas.polygon(&[outer, hole], 7);
    }

    #[test]
    fn test_polygon_many_holes_no_hang() {
        let mut canvas = Canvas::new(200, 200);
        // Outer ring + 20 tiny holes
        let outer = vec![(0, 0), (100, 0), (100, 100), (0, 100)];
        let mut rings = vec![outer];
        for i in 0..20 {
            let x = (i % 5) * 15 + 10;
            let y = (i / 5) * 15 + 10;
            rings.push(vec![(x, y), (x + 5, y), (x + 5, y + 5), (x, y + 5)]);
        }
        let clipped = canvas.clip_polygon(&rings);
        for r in &clipped {
            canvas.polygon(&[r.clone()], 7);
        }
        // Should complete without hanging
    }
}
