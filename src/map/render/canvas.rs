use std::time::Duration;

use super::braille::BrailleBuffer;
use super::earcut_worker::{EarcutWorker, TriangulateError};
use super::frame::MapFrame;
use super::geom::{BresenhamIter, clip_line, sutherland_hodgman_into};
use super::label::LabelBuffer;

use super::VIEWPORT_PADDING;

/// Skip polygons whose vertex count exceeds this — earcut's cost grows
/// fast on large degenerate input, and anything this big after clipping
/// likely indicates pathological geometry.
const POLYGON_VERTEX_CAP: usize = 8000;

/// Per-polygon hard timeout for earcut. Above this we abandon the
/// worker thread (zombie) and skip the polygon. Generous enough to
/// admit legitimate complex coastlines, short enough that a single
/// pathological feature doesn't ruin the frame.
const POLYGON_TRIANGULATE_TIMEOUT: Duration = Duration::from_millis(200);

pub struct Canvas {
    width: usize,
    height: usize,
    buffer: BrailleBuffer,
    labels: LabelBuffer,
    // Scratch buffers reused across polygon() / filled_triangle() calls
    // to avoid per-triangle / per-polygon heap allocations.
    scratch_edge_pixels: Vec<(i32, i32)>,
    scratch_vertices: Vec<[f64; 2]>,
    scratch_hole_indices: Vec<usize>,
    earcut_worker: EarcutWorker,
    // Ping-pong buffers used by sutherland_hodgman_into.
    sh_buf_a: Vec<(i32, i32)>,
    sh_buf_b: Vec<(i32, i32)>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            height,
            buffer: BrailleBuffer::new(width, height),
            labels: LabelBuffer::new(),
            scratch_edge_pixels: Vec::new(),
            scratch_vertices: Vec::new(),
            scratch_hole_indices: Vec::new(),
            earcut_worker: EarcutWorker::new(),
            sh_buf_a: Vec::new(),
            sh_buf_b: Vec::new(),
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

    /// User-overlay variant of [`Canvas::polyline`] that uses
    /// [`BrailleBuffer::set_pixel_punching`] so a line over a fully
    /// saturated cell (e.g. the interior of a water polygon) shows as a
    /// thin path on the cell's existing background instead of flipping
    /// the whole cell to the overlay colour. Sparse cells still get the
    /// OR-merge behaviour of `set_pixel`.
    ///
    /// Used by `Renderer::draw`'s third (overlay) pass; tile features
    /// continue to use the standard [`polyline`](Self::polyline).
    pub fn polyline_punching(&mut self, points: &[(i32, i32)], color: u8) {
        if points.len() < 2 {
            return;
        }
        for i in 0..points.len() - 1 {
            let (x0, y0) = points[i];
            let (x1, y1) = points[i + 1];
            self.draw_line_clipped_punching(x0, y0, x1, y1, color);
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

        // Vertex cap: cheap pre-filter for pathological input. Earcut's
        // cost on degenerate geometry can explode well before reaching
        // anything visually useful at this scale.
        let total_vertices: usize = rings.iter().map(|r| r.len()).sum();
        if total_vertices > POLYGON_VERTEX_CAP {
            log::warn!(
                "polygon skipped: {} vertices ({} rings) exceeds cap {}",
                total_vertices,
                rings.len(),
                POLYGON_VERTEX_CAP
            );
            return;
        }

        self.scratch_vertices.clear();
        self.scratch_hole_indices.clear();

        for &(x, y) in &rings[0] {
            self.scratch_vertices.push([x as f64, y as f64]);
        }

        for ring in &rings[1..] {
            if ring.len() < 3 {
                continue;
            }
            self.scratch_hole_indices.push(self.scratch_vertices.len());
            for &(x, y) in ring {
                self.scratch_vertices.push([x as f64, y as f64]);
            }
        }

        // Triangulate on a dedicated worker thread with a hard timeout.
        // earcut can not only panic on degenerate input (handled inside
        // the worker via silence_panics) but also hang in an infinite
        // loop on pathological self-intersections that survive our
        // Sutherland–Hodgman clip — the timeout is the only protection
        // against the latter.
        let indices = match self.earcut_worker.triangulate(
            self.scratch_vertices.clone(),
            self.scratch_hole_indices.clone(),
            POLYGON_TRIANGULATE_TIMEOUT,
        ) {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => return,
            Err(TriangulateError::TimedOut) => {
                // Capture polygon shape so a future fixture / regression
                // test can target the same pathological input.
                let ring_sizes: Vec<usize> = rings.iter().map(|r| r.len()).collect();
                log::warn!(
                    "polygon dropped: earcut timed out after {:?} \
                     (rings={}, ring_sizes={:?}, total_verts={})",
                    POLYGON_TRIANGULATE_TIMEOUT,
                    rings.len(),
                    ring_sizes,
                    total_vertices,
                );
                return;
            }
            Err(TriangulateError::WorkerDied) => {
                log::warn!("polygon dropped: earcut worker died");
                return;
            }
        };

        let mut i = 0;
        while i + 2 < indices.len() {
            let ia = indices[i];
            let ib = indices[i + 1];
            let ic = indices[i + 2];
            let a = [
                self.scratch_vertices[ia][0] as i32,
                self.scratch_vertices[ia][1] as i32,
            ];
            let b = [
                self.scratch_vertices[ib][0] as i32,
                self.scratch_vertices[ib][1] as i32,
            ];
            let c = [
                self.scratch_vertices[ic][0] as i32,
                self.scratch_vertices[ic][1] as i32,
            ];
            self.filled_triangle(a, b, c, color);
            i += 3;
        }
    }

    /// Clip a polygon to the padded viewport using Sutherland-Hodgman. Each
    /// ring is clipped independently. Writes surviving rings (len >= 3)
    /// into `output` (reusing existing inner Vec capacities) and returns the
    /// number of rings written. Entries past the returned count are left in
    /// place so their capacities can be reused on subsequent calls.
    pub(super) fn clip_polygon_into(
        &mut self,
        rings: &[Vec<(i32, i32)>],
        output: &mut Vec<Vec<(i32, i32)>>,
    ) -> usize {
        let bounds = self.clip_bounds();
        while output.len() < rings.len() {
            output.push(Vec::new());
        }
        let mut kept = 0;
        for ring in rings {
            sutherland_hodgman_into(
                ring,
                bounds,
                &mut self.sh_buf_a,
                &mut self.sh_buf_b,
                &mut output[kept],
            );
            if output[kept].len() >= 3 {
                kept += 1;
            }
        }
        kept
    }

    /// Owned-result variant used by tests. Production code should use
    /// [`Self::clip_polygon_into`] with a reusable output Vec.
    #[cfg(test)]
    pub(super) fn clip_polygon(&mut self, rings: &[Vec<(i32, i32)>]) -> Vec<Vec<(i32, i32)>> {
        let mut output = Vec::new();
        let kept = self.clip_polygon_into(rings, &mut output);
        output.truncate(kept);
        output
    }

    fn clip_bounds(&self) -> (i32, i32, i32, i32) {
        (
            -VIEWPORT_PADDING,
            -VIEWPORT_PADDING,
            self.width as i32 + VIEWPORT_PADDING,
            self.height as i32 + VIEWPORT_PADDING,
        )
    }

    fn draw_line_clipped(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        if let Some((cx0, cy0, cx1, cy1)) = clip_line(self.clip_bounds(), x0, y0, x1, y1) {
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

    fn draw_line_clipped_punching(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        if let Some((cx0, cy0, cx1, cy1)) = clip_line(self.clip_bounds(), x0, y0, x1, y1) {
            self.line_bresenham_punching(cx0, cy0, cx1, cy1, color);
        }
    }

    fn line_bresenham_punching(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        for (x, y) in BresenhamIter::new(x0, y0, x1, y1) {
            if x >= 0 && y >= 0 {
                self.buffer
                    .set_pixel_punching(x as usize, y as usize, color);
            }
        }
    }

    fn filled_triangle(&mut self, a: [i32; 2], b: [i32; 2], c: [i32; 2], color: u8) {
        self.scratch_edge_pixels.clear();

        let bounds = self.clip_bounds();
        for pair in [(&a, &b), (&b, &c), (&c, &a)] {
            if let Some((cx0, cy0, cx1, cy1)) =
                clip_line(bounds, pair.0[0], pair.0[1], pair.1[0], pair.1[1])
            {
                self.scratch_edge_pixels
                    .extend(BresenhamIter::new(cx0, cy0, cx1, cy1));
            }
        }

        if self.scratch_edge_pixels.is_empty() {
            return;
        }

        let h = self.height as i32;
        let w = self.width as i32;
        self.scratch_edge_pixels.retain(|p| p.1 >= 0 && p.1 < h);
        self.scratch_edge_pixels
            .sort_by(|p1, p2| p1.1.cmp(&p2.1).then(p1.0.cmp(&p2.0)));

        // scratch_edge_pixels is sorted by (y asc, x asc) above, so within each
        // same-y run the first and last entries are already the min/max x.
        let mut i = 0;
        while i < self.scratch_edge_pixels.len() {
            let y = self.scratch_edge_pixels[i].1;
            let start = i;
            while i < self.scratch_edge_pixels.len() && self.scratch_edge_pixels[i].1 == y {
                i += 1;
            }
            let min_x = self.scratch_edge_pixels[start].0.max(0);
            let max_x = self.scratch_edge_pixels[i - 1].0.min(w - 1);
            for x in min_x..=max_x {
                self.buffer.set_pixel(x as usize, y as usize, color);
            }
        }
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
    fn test_huge_offscreen_line_no_hang() {
        let mut canvas = Canvas::new(100, 100);
        canvas.polyline(&[(50, 50), (1000000, 1000000)], 7);
    }

    #[test]
    fn test_clip_polygon_method() {
        let mut canvas = Canvas::new(100, 100);
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
