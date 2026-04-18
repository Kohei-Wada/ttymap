//! Renderer — transforms tile features into pixel output.
//! Receives pre-fetched tile data and draws it. Does not know about tile cache.

use std::sync::Arc;

use log::debug;

use super::canvas::Canvas;
use super::frame::MapFrame;
use super::view::VisibleTile;
use crate::styler::{StyleType, Styler};
use crate::tile::{Feature, Point};

/// Pre-collected tile data ready for rendering.
pub struct TileData {
    pub vis: VisibleTile,
    pub layers: Vec<(String, Vec<Feature>)>,
}

pub struct Renderer {
    canvas: Canvas,
    styler: Arc<Styler>,
    width: usize,
    height: usize,
    // Scratch buffers reused across scale_ring calls to avoid per-ring
    // heap allocations. `scratch_line` holds a single scaled ring for the
    // polyline path; `scratch_rings` is a pool of inner Vecs for the fill
    // path (each feature may contain multiple rings).
    scratch_line: Vec<(i32, i32)>,
    scratch_rings: Vec<Vec<(i32, i32)>>,
    scratch_clipped: Vec<Vec<(i32, i32)>>,
}

impl Renderer {
    pub fn new(styler: Arc<Styler>, width: usize, height: usize) -> Self {
        Renderer {
            canvas: Canvas::new(width, height),
            styler,
            width,
            height,
            scratch_line: Vec::new(),
            scratch_rings: Vec::new(),
            scratch_clipped: Vec::new(),
        }
    }

    pub fn set_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.canvas = Canvas::new(width, height);
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }

    /// Render pre-fetched tile data into a MapFrame.
    /// Returns None if no tile data was provided.
    pub fn draw(&mut self, tile_data: &[TileData], zoom: f64) -> Option<MapFrame> {
        // Clear canvas
        self.canvas.clear();
        if let Some(bg) = self.styler.background_color {
            self.canvas.set_background(bg);
        }

        let tiles_found = tile_data.iter().filter(|t| !t.layers.is_empty()).count();
        if tiles_found == 0 {
            return None;
        }

        let total_features: usize = tile_data
            .iter()
            .flat_map(|t| t.layers.iter())
            .map(|(_, f)| f.len())
            .sum();
        debug!("draw: tiles={}, features={}", tiles_found, total_features);

        // Time-budget for drawing
        const RENDER_BUDGET_MS: u64 = 100;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(RENDER_BUDGET_MS);

        // First pass: non-symbol features
        'outer: for td in tile_data {
            let tile_size = td.vis.size;
            for (_, features) in &td.layers {
                for feature in features
                    .iter()
                    .filter(|f| f.style_type != StyleType::Symbol)
                {
                    if std::time::Instant::now() > deadline {
                        debug!("draw: time budget exceeded");
                        break 'outer;
                    }
                    self.draw_feature(&td.vis, feature, tile_size, zoom);
                }
            }
        }

        // Second pass: symbols sorted by sort key
        let mut symbols: Vec<(&VisibleTile, &Feature)> = Vec::new();
        for td in tile_data {
            for (_, features) in &td.layers {
                for f in features {
                    if f.style_type == StyleType::Symbol {
                        symbols.push((&td.vis, f));
                    }
                }
            }
        }
        symbols.sort_by_key(|(_, f)| f.sort);
        for (vis, feature) in &symbols {
            if std::time::Instant::now() > deadline {
                break;
            }
            self.draw_feature(vis, feature, vis.size, zoom);
        }

        let frame = self.canvas.to_map_frame();
        debug!("draw: frame ready ({}x{})", frame.cols, frame.rows);
        Some(frame)
    }

    // ── Feature drawing ───────────────────────────────────────────────────

    fn scale_ring_into(
        out: &mut Vec<(i32, i32)>,
        vis: &VisibleTile,
        ring: &[Point],
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

        let mut last = (i32::MIN, i32::MIN);
        let mut outside = false;

        for p in ring {
            let pt = (
                (vis.pos_x + p.x as f64 / scale) as i32,
                (vis.pos_y + p.y as f64 / scale) as i32,
            );
            if pt == last {
                continue;
            }
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
                    out.push(last);
                }
            }
            out.push(pt);
        }
    }

    fn draw_feature(&mut self, vis: &VisibleTile, feature: &Feature, scale_denom: f64, zoom: f64) {
        if let Some(min_zoom) = feature.min_zoom
            && zoom < min_zoom
        {
            return;
        }
        if let Some(max_zoom) = feature.max_zoom
            && zoom > max_zoom
        {
            return;
        }

        let extent = 4096.0_f64;
        let scale = extent / scale_denom;

        match feature.style_type {
            StyleType::Line => {
                for ring in feature.points.iter() {
                    Self::scale_ring_into(
                        &mut self.scratch_line,
                        vis,
                        ring,
                        scale,
                        self.width,
                        self.height,
                        true,
                    );
                    if self.scratch_line.len() >= 2 {
                        self.canvas.polyline(&self.scratch_line, feature.color);
                    }
                }
            }
            StyleType::Fill => {
                // Reuse scratch_rings as a pool: ensure enough inner Vecs,
                // scale each ring into a slot, then compact surviving rings
                // (len >= 3) to the front.
                while self.scratch_rings.len() < feature.points.len() {
                    self.scratch_rings.push(Vec::new());
                }
                let mut kept = 0;
                for (i, ring) in feature.points.iter().enumerate() {
                    Self::scale_ring_into(
                        &mut self.scratch_rings[i],
                        vis,
                        ring,
                        scale,
                        self.width,
                        self.height,
                        false,
                    );
                    if self.scratch_rings[i].len() >= 3 {
                        if kept != i {
                            self.scratch_rings.swap(kept, i);
                        }
                        kept += 1;
                    }
                }
                if kept == 0 {
                    return;
                }
                let clipped_count = self
                    .canvas
                    .clip_polygon_into(&self.scratch_rings[..kept], &mut self.scratch_clipped);
                if clipped_count > 0 {
                    self.canvas
                        .polygon(&self.scratch_clipped[..clipped_count], feature.color);
                }
            }
            StyleType::Symbol => {
                if let Some(label) = &feature.label
                    && let Some(ring) = feature.points.first()
                    && let Some(pt) = ring.first()
                {
                    let sx = vis.pos_x + pt.x as f64 / scale;
                    let sy = vis.pos_y + pt.y as f64 / scale;
                    if self.canvas.try_place_label(label, sx, sy) {
                        self.canvas
                            .text(label, sx as usize, sy as usize, feature.color);
                    }
                }
            }
        }
    }

    pub(super) fn draw_order(zoom: f64) -> Vec<&'static str> {
        if zoom < 2.0 {
            vec!["admin", "water", "country_label", "marine_label"]
        } else {
            vec![
                "landuse",
                "landuse_overlay",
                "water",
                "waterway",
                "marine_label",
                "aeroway",
                "building",
                "road",
                "admin",
                "country_label",
                "state_label",
                "water_label",
                "place_label",
                "rail_station_label",
                "airport_label",
                "poi_label",
                "road_label",
                "housenum_label",
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_order_low_zoom() {
        let order = Renderer::draw_order(1.5);
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn test_draw_order_high_zoom() {
        let order = Renderer::draw_order(5.0);
        assert!(order.len() >= 18);
    }

    #[test]
    fn test_draw_empty_returns_none() {
        let styler = Arc::new(Styler::new("dark"));
        let mut renderer = Renderer::new(styler, 80, 40);
        assert!(renderer.draw(&[], 1.0).is_none());
    }
}
