//! Renderer — transforms tile features into pixel output.
//!
//! Features arrive from `tile/` as raw MVT data (layer name + properties +
//! geometry). The renderer resolves each feature against the current `Styler`
//! to decide color / min-max zoom / whether it's a Symbol, and extracts label
//! text using the configured `language`. Keeping this at render time (rather
//! than baking it into `Feature` at decode time) means the tile cache does not
//! need to be flushed when the palette or style preset changes.

use std::sync::Arc;

use log::debug;

use super::canvas::Canvas;
use super::frame::MapFrame;
use super::view::VisibleTile;
use crate::map::styler::{StyleRule, StyleType, Styler};
use crate::map::tile::decode::{Feature, TilePoint};
use crate::map::tile::property::{extract_label, extract_sort};

/// Pre-collected tile data ready for rendering.
pub struct TileData {
    pub vis: VisibleTile,
    pub layers: Vec<(String, Vec<Feature>)>,
}

/// Scratch buffers reused across `draw_non_symbol` calls within a
/// single frame. Collecting them here avoids per-ring heap
/// allocations and keeps the draw helper's signature short.
///
/// - `line` holds a single scaled ring for the polyline path.
/// - `rings` is a pool of inner Vecs for the fill path; each feature
///   may contain multiple rings and we reuse slots across features.
/// - `clipped` receives the Sutherland–Hodgman-clipped polygon
///   produced by `Canvas::clip_polygon_into`.
struct Scratches {
    line: Vec<(i32, i32)>,
    rings: Vec<Vec<(i32, i32)>>,
    clipped: Vec<Vec<(i32, i32)>>,
}

impl Scratches {
    fn new() -> Self {
        Self {
            line: Vec::new(),
            rings: Vec::new(),
            clipped: Vec::new(),
        }
    }
}

/// Per-call draw context bundling everything `draw_non_symbol` needs
/// to mutate (`canvas`, `scratches`) and the viewport dimensions it
/// reads. Constructed fresh per feature so the borrow only covers one
/// call and doesn't fight the `&self.styler` borrow held by the
/// enclosing rule loop.
struct DrawCtx<'a> {
    canvas: &'a mut Canvas,
    scratches: &'a mut Scratches,
    width: usize,
    height: usize,
}

pub struct Renderer {
    canvas: Canvas,
    styler: Arc<Styler>,
    language: String,
    width: usize,
    height: usize,
    scratches: Scratches,
}

/// A symbol feature that survived the first pass. Holds the resolved
/// color and sort key by value (not a reference into `self.styler`) so
/// the symbols Vec doesn't extend the styler borrow into the draw loop.
struct ResolvedSymbol<'a> {
    vis: &'a VisibleTile,
    feature: &'a Feature,
    color: u8,
    sort: i64,
}

impl Renderer {
    pub fn new(styler: Arc<Styler>, language: String, width: usize, height: usize) -> Self {
        Renderer {
            canvas: Canvas::new(width, height),
            styler,
            language,
            width,
            height,
            scratches: Scratches::new(),
        }
    }

    pub fn set_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.canvas = Canvas::new(width, height);
    }

    /// Replace the active `Styler`. Subsequent `draw` calls use the
    /// new theme immediately; in-flight tile decodes are unaffected
    /// (they don't consult the styler).
    pub fn set_styler(&mut self, styler: Arc<Styler>) {
        self.styler = styler;
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }

    /// Render pre-fetched tile data into a `MapFrame`.
    ///
    /// Always returns `Some`: if no tiles are loaded yet (e.g. panning into
    /// an un-fetched area), we still emit a background-only frame so the
    /// coords / scale-bar / place overlays keep updating with the new
    /// `center` and `zoom`. Without this, the UI would look frozen while
    /// tiles are in flight.
    pub fn draw(&mut self, tile_data: &[TileData], zoom: f64) -> Option<MapFrame> {
        // Clear canvas
        self.canvas.clear();
        if let Some(bg) = self.styler.background_color {
            self.canvas.set_background(bg);
        }

        let tiles_found = tile_data.iter().filter(|t| !t.layers.is_empty()).count();
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

        // First pass: non-symbol features draw immediately.
        // Symbol features are collected for a second pass so they can be
        // drawn on top in `sort` order.
        let mut symbols: Vec<ResolvedSymbol> = Vec::new();

        'outer: for td in tile_data {
            let tile_size = td.vis.size;
            for (_, features) in &td.layers {
                for feature in features {
                    if std::time::Instant::now() > deadline {
                        debug!("draw: time budget exceeded");
                        break 'outer;
                    }
                    let Some(rule) = self
                        .styler
                        .get_style_for(&feature.layer_name, &feature.properties)
                    else {
                        continue;
                    };
                    if let Some(min_zoom) = rule.min_zoom
                        && zoom < min_zoom
                    {
                        continue;
                    }
                    if let Some(max_zoom) = rule.max_zoom
                        && zoom > max_zoom
                    {
                        continue;
                    }

                    if rule.style_type == StyleType::Symbol {
                        symbols.push(ResolvedSymbol {
                            vis: &td.vis,
                            feature,
                            color: rule.color,
                            sort: extract_sort(&feature.properties),
                        });
                    } else {
                        let mut ctx = DrawCtx {
                            canvas: &mut self.canvas,
                            scratches: &mut self.scratches,
                            width: self.width,
                            height: self.height,
                        };
                        Self::draw_non_symbol(&mut ctx, &td.vis, feature, rule, tile_size);
                    }
                }
            }
        }

        symbols.sort_by_key(|s| s.sort);
        for resolved in &symbols {
            if std::time::Instant::now() > deadline {
                break;
            }
            self.draw_symbol(resolved);
        }

        let frame = self.canvas.to_map_frame();
        debug!("draw: frame ready ({}x{})", frame.cols, frame.rows);
        Some(frame)
    }

    // ── Feature drawing ───────────────────────────────────────────────────

    /// Project tile-local points into integer screen pixels and append
    /// them to `out`, optionally clipping against the padded viewport.
    ///
    /// Coordinate pipeline for each point: `TilePoint` (i32, 0..extent)
    /// → f64 screen offset relative to `vis.pos_{x,y}` → final screen
    /// pixel (i32). The f64 step is where we spend precision for the
    /// divide; the final `as i32` rounds toward zero, matching the
    /// tolerance of the braille pixel grid. `inv_scale = 1.0 / scale`
    /// is precomputed so each point needs a multiply rather than a
    /// divide.
    fn scale_ring_into(
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

    /// Draw Line / Fill features. `StyleType::Symbol` is handled separately.
    fn draw_non_symbol(
        ctx: &mut DrawCtx,
        vis: &VisibleTile,
        feature: &Feature,
        rule: &StyleRule,
        scale_denom: f64,
    ) {
        let extent = 4096.0_f64;
        let scale = extent / scale_denom;
        let (width, height) = (ctx.width, ctx.height);

        match rule.style_type {
            StyleType::Line => {
                for ring in feature.points.iter() {
                    Self::scale_ring_into(
                        &mut ctx.scratches.line,
                        vis,
                        ring,
                        scale,
                        width,
                        height,
                        true,
                    );
                    if ctx.scratches.line.len() >= 2 {
                        ctx.canvas.polyline(&ctx.scratches.line, rule.color);
                    }
                }
            }
            StyleType::Fill => {
                // Reuse scratches.rings as a pool: ensure enough inner
                // Vecs, scale each ring into a slot, then compact
                // surviving rings (len >= 3) to the front.
                let rings = &mut ctx.scratches.rings;
                while rings.len() < feature.points.len() {
                    rings.push(Vec::new());
                }
                let mut kept = 0;
                for (i, ring) in feature.points.iter().enumerate() {
                    Self::scale_ring_into(&mut rings[i], vis, ring, scale, width, height, false);
                    if rings[i].len() >= 3 {
                        if kept != i {
                            rings.swap(kept, i);
                        }
                        kept += 1;
                    }
                }
                if kept == 0 {
                    return;
                }
                let clipped_count = ctx
                    .canvas
                    .clip_polygon_into(&rings[..kept], &mut ctx.scratches.clipped);
                if clipped_count > 0 {
                    ctx.canvas
                        .polygon(&ctx.scratches.clipped[..clipped_count], rule.color);
                }
            }
            StyleType::Symbol => {}
        }
    }

    fn draw_symbol(&mut self, resolved: &ResolvedSymbol) {
        let feature = resolved.feature;
        let vis = resolved.vis;
        let extent = 4096.0_f64;
        let scale = extent / vis.size;

        let Some(label) = extract_label(&feature.properties, &self.language) else {
            return;
        };
        let Some(ring) = feature.points.first() else {
            return;
        };
        let Some(pt) = ring.first() else {
            return;
        };
        let sx = vis.pos_x + pt.x as f64 / scale;
        let sy = vis.pos_y + pt.y as f64 / scale;
        if self.canvas.try_place_label(&label, sx, sy) {
            self.canvas
                .text(&label, sx as usize, sy as usize, resolved.color);
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
    fn test_draw_empty_still_emits_frame() {
        // Empty tile data still produces a (background-only) frame so
        // overlays (coords / scale bar / place) keep refreshing while
        // tiles are in flight. Cf. `draw` doc comment.
        let styler = Arc::new(Styler::new(crate::color_palette::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 80, 40);
        assert!(renderer.draw(&[], 1.0).is_some());
    }
}
