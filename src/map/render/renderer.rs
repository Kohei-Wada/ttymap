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
    pub layers: Vec<LayerData>,
}

/// One layer's worth of features plus the MVT `extent` reported by that
/// layer. Extent is per-layer (not a tile-wide constant) and varies
/// across tile sources — `mapscii.me` uses 4096 but other OpenMapTiles
/// deployments use 2048 / 8192. The draw path needs it to compute the
/// tile-coord → screen-pixel scale.
pub struct LayerData {
    pub name: String,
    pub extent: u32,
    pub features: Vec<Feature>,
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
    extent: f64,
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
    pub fn draw(
        &mut self,
        tile_data: &[TileData],
        zoom: f64,
        center: crate::geo::LonLat,
        overlays: &[crate::map::render::overlay::UserPolyline],
    ) -> Option<MapFrame> {
        // Clear canvas
        self.canvas.clear();
        if let Some(bg) = self.styler.background_color {
            self.canvas.set_background(bg);
        }

        let tiles_found = tile_data.iter().filter(|t| !t.layers.is_empty()).count();
        let total_features: usize = tile_data
            .iter()
            .flat_map(|t| t.layers.iter())
            .map(|l| l.features.len())
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
            for layer in &td.layers {
                let extent = layer.extent as f64;
                for feature in &layer.features {
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
                            extent,
                        });
                    } else {
                        let mut ctx = DrawCtx {
                            canvas: &mut self.canvas,
                            scratches: &mut self.scratches,
                            width: self.width,
                            height: self.height,
                        };
                        Self::draw_non_symbol(&mut ctx, &td.vis, feature, rule, tile_size, extent);
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

        // Third pass: user overlays from Lua plugins. Drawn on the same
        // canvas as tile features so dots OR-merge (BrailleBuffer::set_pixel
        // |= bit) and the overlay's fg wins per-cell (set_pixel overwrites
        // fg_buf). Same render budget applies — pathological coord lists
        // stop early instead of starving the next frame.
        let mut buf: Vec<(i32, i32)> = std::mem::take(&mut self.scratches.line);
        for poly in overlays {
            if std::time::Instant::now() > deadline {
                break;
            }
            buf.clear();
            for ll in &poly.coords {
                buf.push(crate::map::render::overlay::ll_to_subpixel(
                    *ll,
                    center,
                    zoom,
                    self.width,
                    self.height,
                ));
            }
            if buf.len() >= 2 {
                self.canvas.polyline_punching(&buf, poly.color);
            }
        }
        self.scratches.line = buf;

        let frame = self.canvas.to_map_frame();
        debug!("draw: frame ready ({}x{})", frame.cols, frame.rows);
        Some(frame)
    }

    // ── Polygon multi-ring classification (issue #101) ────────────────

    /// Surveyor's-formula signed area of a closed ring, accumulated in
    /// `i64` to avoid overflow at typical screen-pixel magnitudes.
    /// In MVT tile coords (Y-down) and screen coords (also Y-down), a
    /// CW ring has positive signed area and is the **exterior**; CCW
    /// (negative area) is a **hole**.
    fn signed_area(ring: &[(i32, i32)]) -> i64 {
        if ring.len() < 3 {
            return 0;
        }
        let mut acc: i64 = 0;
        for i in 0..ring.len() {
            let (x0, y0) = ring[i];
            let (x1, y1) = ring[(i + 1) % ring.len()];
            acc += (x0 as i64) * (y1 as i64) - (x1 as i64) * (y0 as i64);
        }
        acc
    }

    /// Group a flat ring list into polygon groups. Each group starts
    /// at an exterior ring (signed area > 0) and runs through any
    /// following interior rings until the next exterior. Leading
    /// interior rings (malformed input) are dropped. Returns
    /// half-open ranges into the input slice.
    fn classify_polygon_groups(rings: &[Vec<(i32, i32)>]) -> Vec<std::ops::Range<usize>> {
        let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
        for (i, ring) in rings.iter().enumerate() {
            if Self::signed_area(ring) > 0 {
                if let Some(last) = groups.last_mut() {
                    last.end = i;
                }
                groups.push(i..i + 1);
            }
        }
        if let Some(last) = groups.last_mut() {
            last.end = rings.len();
        }
        groups
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

    /// Draw Line / Fill features. `StyleType::Symbol` is handled separately.
    fn draw_non_symbol(
        ctx: &mut DrawCtx,
        vis: &VisibleTile,
        feature: &Feature,
        rule: &StyleRule,
        scale_denom: f64,
        extent: f64,
    ) {
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
                // A single MVT polygon feature may pack multiple
                // non-overlapping outer rings (multi-polygon, common
                // for "all lakes / all parks in tile" features).
                // Group rings by winding (issue #101) and draw each
                // outer-with-its-holes independently — otherwise
                // earcut treats the second outer as a hole of the
                // first, mangling the fill.
                for group in Self::classify_polygon_groups(&rings[..kept]) {
                    let clipped_count = ctx.canvas.clip_polygon_into(
                        &rings[group.start..group.end],
                        &mut ctx.scratches.clipped,
                    );
                    if clipped_count > 0 {
                        ctx.canvas
                            .polygon(&ctx.scratches.clipped[..clipped_count], rule.color);
                    }
                }
            }
            StyleType::Symbol => {}
        }
    }

    fn draw_symbol(&mut self, resolved: &ResolvedSymbol) {
        let feature = resolved.feature;
        let vis = resolved.vis;
        let scale = resolved.extent / vis.size;

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
        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::TilePoint;

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
        Renderer::scale_ring_into(&mut out, &vis, &ring, 1.0, 100, 100, true);

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
        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::TilePoint;

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
        Renderer::scale_ring_into(&mut out, &vis, &ring, 1.0, 100, 100, true);
        assert_eq!(out, vec![(10, 10), (20, 20), (30, 30)]);
    }

    // ── Multi-polygon ring classification (issue #101) ────────────────

    /// MVT spec: in tile (Y-down) coords, a CW ring has positive
    /// signed area and is the **exterior**; CCW (negative area) is a
    /// hole.
    #[test]
    fn signed_area_positive_for_clockwise_ring_in_y_down() {
        // Square traversed top-left → top-right → bottom-right →
        // bottom-left → close. CW visually in Y-down (screen) coords.
        let ring = vec![(0, 0), (10, 0), (10, 10), (0, 10)];
        assert!(
            Renderer::signed_area(&ring) > 0,
            "CW ring in Y-down coords must report positive signed area"
        );
    }

    #[test]
    fn signed_area_negative_for_counterclockwise_ring_in_y_down() {
        // Same square, reversed → CCW in Y-down.
        let ring = vec![(0, 0), (0, 10), (10, 10), (10, 0)];
        assert!(
            Renderer::signed_area(&ring) < 0,
            "CCW ring in Y-down coords must report negative signed area"
        );
    }

    #[test]
    fn signed_area_zero_for_degenerate_ring() {
        let ring = vec![(0, 0), (10, 0)]; // < 3 points
        assert_eq!(Renderer::signed_area(&ring), 0);
    }

    /// Empty input → no groups.
    #[test]
    fn classify_polygon_groups_empty_input_yields_no_groups() {
        let rings: Vec<Vec<(i32, i32)>> = Vec::new();
        assert!(Renderer::classify_polygon_groups(&rings).is_empty());
    }

    /// Single outer ring → one group spanning the whole slice.
    #[test]
    fn classify_polygon_groups_single_outer() {
        let rings = vec![vec![(0, 0), (10, 0), (10, 10), (0, 10)]];
        assert_eq!(Renderer::classify_polygon_groups(&rings), vec![0..1]);
    }

    /// Outer + hole → one group [0..2].
    #[test]
    fn classify_polygon_groups_outer_with_hole() {
        let rings = vec![
            vec![(0, 0), (100, 0), (100, 100), (0, 100)], // CW = outer
            vec![(20, 20), (20, 40), (40, 40), (40, 20)], // CCW = hole
        ];
        assert_eq!(Renderer::classify_polygon_groups(&rings), vec![0..2]);
    }

    /// Two disjoint outer rings (multi-polygon) → two separate
    /// groups. This is the case the pre-fix renderer mishandled — it
    /// treated the second outer as a hole of the first.
    #[test]
    fn classify_polygon_groups_two_outers_are_two_groups() {
        let rings = vec![
            vec![(0, 0), (10, 0), (10, 10), (0, 10)],
            vec![(50, 50), (60, 50), (60, 60), (50, 60)],
        ];
        assert_eq!(Renderer::classify_polygon_groups(&rings), vec![0..1, 1..2]);
    }

    /// Mixed: outer, hole, outer, hole → two groups, each [outer,
    /// hole].
    #[test]
    fn classify_polygon_groups_outer_hole_outer_hole() {
        let rings = vec![
            vec![(0, 0), (100, 0), (100, 100), (0, 100)], // outer A
            vec![(20, 20), (20, 40), (40, 40), (40, 20)], // hole of A
            vec![(200, 0), (300, 0), (300, 100), (200, 100)], // outer B
            vec![(220, 20), (220, 40), (240, 40), (240, 20)], // hole of B
        ];
        assert_eq!(Renderer::classify_polygon_groups(&rings), vec![0..2, 2..4]);
    }

    /// Leading hole (malformed input) is skipped; valid outer behind
    /// it still becomes a group.
    #[test]
    fn classify_polygon_groups_drops_leading_holes() {
        let rings = vec![
            vec![(0, 0), (0, 10), (10, 10), (10, 0)], // CCW: hole, no parent
            vec![(50, 50), (60, 50), (60, 60), (50, 60)], // CW: outer
        ];
        assert_eq!(Renderer::classify_polygon_groups(&rings), vec![1..2]);
    }

    /// Regression for issue #101. A POLYGON feature that packs two
    /// disjoint outer rings (multi-polygon, common for "all lakes /
    /// all parks in tile" features) must fill **both** screen
    /// regions. The pre-fix renderer fed the flat ring list straight
    /// to earcut, which mistreated the second outer as a hole of the
    /// first → only one polygon (or none) drew.
    #[test]
    fn fill_renders_both_outers_of_a_multi_polygon_feature() {
        use std::collections::HashMap;

        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::{Feature, TilePoint};

        // Canvas 320×320 px (160×80 cells). vis.size=256, extent=4096
        // → scale = 4096/256 = 16, so tile-coord 16 = 1 screen pixel.
        let vis = VisibleTile {
            x: 0,
            y: 0,
            z: 14,
            pos_x: 0.0,
            pos_y: 0.0,
            size: 256.0,
        };

        // Two CW (= outer in Y-down) rings packed into one feature.
        // Ring A occupies upper-left; ring B occupies lower-right.
        let cw_square = |x0: i32, y0: i32, x1: i32, y1: i32| {
            vec![
                TilePoint { x: x0, y: y0 },
                TilePoint { x: x1, y: y0 },
                TilePoint { x: x1, y: y1 },
                TilePoint { x: x0, y: y1 },
            ]
        };
        let points = Arc::new(vec![
            cw_square(100, 100, 1000, 1000),
            cw_square(2000, 2000, 3000, 3000),
        ]);
        let feature = Feature {
            layer_name: Arc::from("water"),
            properties: Arc::new(HashMap::new()),
            points,
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
        };
        let tile = TileData {
            vis,
            layers: vec![LayerData {
                name: "water".to_string(),
                extent: 4096,
                features: vec![feature],
            }],
        };

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 320, 320);
        let frame = renderer
            .draw(
                &[tile],
                14.0,
                crate::geo::LonLat { lon: 0.0, lat: 0.0 },
                &[],
            )
            .expect("frame");

        // Ring A's screen bbox in pixels: (6,6)..(62,62) → cells
        // (3,1)..(31,15). Ring B's: (125,125)..(187,187) → cells
        // (62,31)..(93,46). Sample a cell deep inside each region.
        let cell_at = |col: usize, row: usize| {
            let idx = row * frame.cols as usize + col;
            frame.cells[idx].ch
        };
        // Empty braille char is U+2800 ('⠀'). Anything else means
        // some pixel was drawn.
        const EMPTY: char = '⠀';
        assert_ne!(
            cell_at(15, 7),
            EMPTY,
            "ring A (upper-left outer) must be filled"
        );
        assert_ne!(
            cell_at(75, 38),
            EMPTY,
            "ring B (lower-right outer) must be filled too — pre-fix \
             treats it as a hole of A and leaves it empty"
        );
    }

    #[test]
    fn test_draw_empty_still_emits_frame() {
        // Empty tile data still produces a (background-only) frame so
        // overlays (coords / scale bar / place) keep refreshing while
        // tiles are in flight. Cf. `draw` doc comment.
        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 80, 40);
        assert!(
            renderer
                .draw(&[], 1.0, crate::geo::LonLat { lon: 0.0, lat: 0.0 }, &[])
                .is_some()
        );
    }

    /// Regression test for issue #100. A polygon described in tile-local
    /// coords scaled to its layer's `extent` must render to the exact
    /// same screen-space output regardless of which extent the source
    /// reports (4096 / 2048 / 8192 are all valid in MVT). Previously the
    /// renderer hardcoded extent=4096, so non-default sources rendered
    /// at the wrong scale.
    #[test]
    fn extent_invariance_across_layer_extents() {
        use std::collections::HashMap;

        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::{Feature, TilePoint};

        // A diamond polygon expressed as fractions of `extent`. At any
        // valid extent, the resulting screen-space polygon is identical.
        fn make(extent: u32) -> TileData {
            let e = extent as i32;
            let p = |fx: i32, fy: i32| TilePoint {
                x: e * fx / 10,
                y: e * fy / 10,
            };
            let points = Arc::new(vec![vec![p(2, 5), p(5, 2), p(8, 5), p(5, 8), p(2, 5)]]);
            let feature = Feature {
                layer_name: Arc::from("water"),
                properties: Arc::new(HashMap::new()),
                points,
                min_x: 0.0,
                max_x: 0.0,
                min_y: 0.0,
                max_y: 0.0,
            };
            TileData {
                vis: VisibleTile {
                    x: 0,
                    y: 0,
                    z: 14,
                    pos_x: 0.0,
                    pos_y: 0.0,
                    size: 256.0,
                },
                layers: vec![LayerData {
                    name: "water".to_string(),
                    extent,
                    features: vec![feature],
                }],
            }
        }

        // Canvas dims are in *pixels* (braille sub-pixels), not cells —
        // 320×320 px ≈ 160×80 cells. Comfortably larger than `vis.size`
        // so the test geometry lands inside the frame.
        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let cells = |extent: u32| -> Vec<(char, u8, u8)> {
            let mut r = Renderer::new(Arc::clone(&styler), "en".to_string(), 320, 320);
            let f = r
                .draw(
                    &[make(extent)],
                    14.0,
                    crate::geo::LonLat { lon: 0.0, lat: 0.0 },
                    &[],
                )
                .expect("frame");
            f.cells.iter().map(|c| (c.ch, c.fg, c.bg)).collect()
        };

        // Sanity: at least one non-empty cell — without this guard the
        // test could pass trivially if everything fell off-canvas.
        let baseline = cells(4096);
        let nonempty = baseline.iter().filter(|(ch, _, _)| *ch != '⠀').count();
        assert!(
            nonempty > 0,
            "test setup: expected polygon to render some cells"
        );

        assert_eq!(
            baseline,
            cells(2048),
            "extent=2048 must render identically to extent=4096"
        );
        assert_eq!(
            baseline,
            cells(8192),
            "extent=8192 must render identically to extent=4096"
        );
    }

    /// Same invariance check for the symbol pass (`draw_symbol`),
    /// which had its own hardcoded `extent = 4096.0`.
    #[test]
    fn extent_invariance_for_symbols() {
        use std::collections::HashMap;

        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::{Feature, TilePoint};
        use crate::map::tile::property::PropertyValue;

        // `place_label` in the mapscii schema renders as a Symbol whose
        // text comes from `name`. A single point near tile center.
        fn make(extent: u32) -> TileData {
            let e = extent as i32;
            let mut props: HashMap<Arc<str>, PropertyValue> = HashMap::new();
            props.insert(Arc::from("name"), PropertyValue::String("X".into()));
            props.insert(Arc::from("type"), PropertyValue::String("city".into()));
            let feature = Feature {
                layer_name: Arc::from("place_label"),
                properties: Arc::new(props),
                points: Arc::new(vec![vec![TilePoint { x: e / 2, y: e / 2 }]]),
                min_x: 0.0,
                max_x: 0.0,
                min_y: 0.0,
                max_y: 0.0,
            };
            TileData {
                vis: VisibleTile {
                    x: 0,
                    y: 0,
                    z: 14,
                    pos_x: 0.0,
                    pos_y: 0.0,
                    size: 256.0,
                },
                layers: vec![LayerData {
                    name: "place_label".to_string(),
                    extent,
                    features: vec![feature],
                }],
            }
        }

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let cells = |extent: u32| -> Vec<(char, u8, u8)> {
            let mut r = Renderer::new(Arc::clone(&styler), "en".to_string(), 320, 320);
            let f = r
                .draw(
                    &[make(extent)],
                    14.0,
                    crate::geo::LonLat { lon: 0.0, lat: 0.0 },
                    &[],
                )
                .expect("frame");
            f.cells.iter().map(|c| (c.ch, c.fg, c.bg)).collect()
        };

        // Sanity: confirm the symbol pass actually placed the label.
        let baseline = cells(4096);
        let label_cells = baseline.iter().filter(|(ch, _, _)| *ch == 'X').count();
        assert!(
            label_cells > 0,
            "test setup: expected the symbol pass to draw the label"
        );

        assert_eq!(baseline, cells(2048));
        assert_eq!(baseline, cells(8192));
    }

    /// User overlay polylines must paint on the *same* BrailleBuffer as
    /// tile features. Two non-empty cells along the projected polyline is
    /// the minimum signal that the overlay reached the canvas.
    #[test]
    fn user_overlay_polyline_renders_braille_dots() {
        use crate::geo::LonLat;
        use crate::map::render::overlay::UserPolyline;

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 320, 320);
        let center = LonLat { lon: 0.0, lat: 0.0 };
        let overlays = vec![UserPolyline {
            coords: vec![
                LonLat {
                    lon: -1.0,
                    lat: 1.0,
                },
                LonLat {
                    lon: 1.0,
                    lat: -1.0,
                },
            ],
            color: 7,
        }];
        let frame = renderer.draw(&[], 6.0, center, &overlays).expect("frame");

        const EMPTY: char = '\u{2800}';
        let drawn = frame.cells.iter().filter(|c| c.ch != EMPTY).count();
        assert!(
            drawn >= 2,
            "overlay polyline must paint at least two non-empty braille cells, got {drawn}"
        );
    }

    /// Bug fix: an overlay polyline crossing a fully-saturated cell
    /// (e.g. the interior of a water polygon) must render as a thin
    /// shape, not as a 2-column-wide block of overlay colour.
    ///
    /// Probe: render a Fill feature large enough to fully saturate at
    /// least one cell, then draw an overlay polyline that crosses that
    /// cell. The combined frame's cell (at the crossing) must NOT be
    /// `⣿` — its mask must contain only the overlay's subpixel(s),
    /// strictly fewer than 8 dots.
    #[test]
    fn overlay_punches_through_saturated_tile_fills() {
        use std::collections::HashMap;

        use crate::geo::LonLat;
        use crate::map::render::overlay::UserPolyline;
        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::{Feature, TilePoint};

        // Big water rectangle covering the full tile so its interior
        // cells saturate.
        let cw_square = |x0: i32, y0: i32, x1: i32, y1: i32| {
            vec![
                TilePoint { x: x0, y: y0 },
                TilePoint { x: x1, y: y0 },
                TilePoint { x: x1, y: y1 },
                TilePoint { x: x0, y: y1 },
            ]
        };
        let make_water_tile = || TileData {
            vis: VisibleTile {
                x: 0,
                y: 0,
                z: 14,
                pos_x: 0.0,
                pos_y: 0.0,
                size: 256.0,
            },
            layers: vec![LayerData {
                name: "water".to_string(),
                extent: 4096,
                features: vec![Feature {
                    layer_name: Arc::from("water"),
                    properties: Arc::new(HashMap::new()),
                    points: Arc::new(vec![cw_square(0, 0, 4096, 4096)]),
                    min_x: 0.0,
                    max_x: 0.0,
                    min_y: 0.0,
                    max_y: 0.0,
                }],
            }],
        };

        // Overlay polyline crossing the centre of the saturated water
        // area. World coords near (0, 0) at zoom 6 land in the visible
        // canvas.
        let center = LonLat { lon: 0.0, lat: 0.0 };
        let overlay = UserPolyline {
            coords: vec![
                LonLat {
                    lon: -0.5,
                    lat: 0.0,
                },
                LonLat { lon: 0.5, lat: 0.0 },
            ],
            color: 11,
        };

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 320, 320);

        // Tile-only frame: confirm the fixture does saturate at least
        // one cell (sanity check). Otherwise the test is vacuous.
        let tile_only = renderer
            .draw(&[make_water_tile()], 6.0, center, &[])
            .expect("frame");
        let saturated_count = tile_only.cells.iter().filter(|c| c.ch == '⣿').count();
        assert!(
            saturated_count > 0,
            "fixture: water fill must produce at least one saturated cell, \
             got {saturated_count}"
        );

        // Combined frame: overlay should "punch through" any saturated
        // cell it crosses.
        let combined = renderer
            .draw(&[make_water_tile()], 6.0, center, &[overlay])
            .expect("frame");

        // Find a cell along the overlay's path where the tile-only
        // version was saturated. That cell in the combined frame must
        // have a non-⣿ char with a strict-subset dot mask.
        let mut found = false;
        for (i, (t, b)) in tile_only
            .cells
            .iter()
            .zip(combined.cells.iter())
            .enumerate()
        {
            if t.ch == '⣿' && b.ch != '⣿' {
                let mask_b = (b.ch as u32) - 0x2800;
                assert!(
                    mask_b != 0xFF,
                    "combined cell {i} must not be saturated, got mask 0x{mask_b:02x}"
                );
                assert!(
                    mask_b.count_ones() < 8,
                    "punching cell {i} must have strictly fewer than 8 dots, \
                     got {} dots",
                    mask_b.count_ones()
                );
                assert_eq!(b.fg, 11, "combined cell {i} fg must be the overlay colour");
                let expected_bg = t.fg;
                assert_eq!(
                    b.bg, expected_bg,
                    "combined cell {i} bg must inherit the cell's prior fg \
                     (water fill colour, here {expected_bg}) so OFF subpixels \
                     render against the underlying fill — not the global bg"
                );
                found = true;
                break;
            }
        }
        assert!(
            found,
            "no overlay-on-saturated-cell crossing detected — adjust the \
             overlay's coords or zoom so the line passes through a saturated \
             interior cell"
        );
    }

    /// An empty overlays slice produces the same frame as a no-overlay
    /// baseline. Defensive — guards against accidental side-effects in
    /// the third pass (e.g. clearing the canvas, mutating shared state).
    #[test]
    fn empty_overlays_match_baseline() {
        use crate::geo::LonLat;

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut r = Renderer::new(Arc::clone(&styler), "en".to_string(), 80, 40);
        let center = LonLat { lon: 0.0, lat: 0.0 };
        let baseline = r.draw(&[], 4.0, center, &[]).expect("frame");
        let with_empty = r.draw(&[], 4.0, center, &[]).expect("frame");
        assert_eq!(
            baseline.cells.iter().map(|c| c.ch).collect::<Vec<_>>(),
            with_empty.cells.iter().map(|c| c.ch).collect::<Vec<_>>(),
        );
    }

    /// Overlay polylines always replace the cell's dot mask, never
    /// OR-merge — see `BrailleBuffer::set_pixel_punching`. So when an
    /// overlay crosses a tile-feature line, the cells at the crossing
    /// carry only the overlay's bits in the overlay's colour. Without
    /// this rule the cells would carry both sets of bits but with the
    /// overlay's foreground (last-writer-wins on `fg_buf`), tinting the
    /// tile feature with the overlay colour — visible to users as a
    /// "halo" of overlay colour around the line wherever it passes
    /// through tile content.
    #[test]
    fn overlay_replaces_tile_feature_dots_at_crossing() {
        use std::collections::HashMap;

        use crate::geo::LonLat;
        use crate::map::render::overlay::UserPolyline;
        use crate::map::render::view::VisibleTile;
        use crate::map::tile::decode::{Feature, TilePoint};
        use crate::map::tile::property::PropertyValue;

        // Horizontal road across the tile centre.
        let make_tile = || {
            let mut props: HashMap<Arc<str>, PropertyValue> = HashMap::new();
            props.insert(
                Arc::from("class"),
                PropertyValue::String(Arc::from("motorway")),
            );
            let road_feature = Feature {
                layer_name: Arc::from("road"),
                properties: Arc::new(props),
                points: Arc::new(vec![vec![
                    TilePoint { x: 0, y: 2048 },
                    TilePoint { x: 4096, y: 2048 },
                ]]),
                min_x: 0.0,
                max_x: 0.0,
                min_y: 0.0,
                max_y: 0.0,
            };
            TileData {
                vis: VisibleTile {
                    x: 32,
                    y: 32,
                    z: 6,
                    pos_x: 0.0,
                    pos_y: 0.0,
                    size: 256.0,
                },
                layers: vec![LayerData {
                    name: "road".to_string(),
                    extent: 4096,
                    features: vec![road_feature],
                }],
            }
        };

        let center = LonLat { lon: 0.0, lat: 0.0 };
        // Vertical overlay at lon=0, lat ∈ [0.5°, 0.9°] — same range as
        // the old OR-merge test, which confirmed the geometry intersects.
        let overlay = UserPolyline {
            coords: vec![LonLat { lon: 0.0, lat: 0.9 }, LonLat { lon: 0.0, lat: 0.5 }],
            color: 11,
        };

        let styler = Arc::new(Styler::new(crate::theme::ThemeId::Dark));
        let mut renderer = Renderer::new(styler, "en".to_string(), 320, 320);

        let tile_only = renderer
            .draw(&[make_tile()], 6.0, center, &[])
            .expect("tile-only frame");
        let overlay_only = renderer
            .draw(&[], 6.0, center, &[overlay.clone()])
            .expect("overlay-only frame");
        let combined = renderer
            .draw(&[make_tile()], 6.0, center, &[overlay])
            .expect("combined frame");

        // For every cell where both the road and the overlay painted,
        // verify the combined frame's mask equals the overlay-only mask
        // (always-replace), not the bitwise union (OR-merge). At least
        // one such overlap cell must exist to make the test non-vacuous.
        const EMPTY: char = '\u{2800}';
        let mut overlap_count = 0usize;
        for i in 0..tile_only.cells.len() {
            let t = &tile_only.cells[i];
            let o = &overlay_only.cells[i];
            let b = &combined.cells[i];
            if t.ch == EMPTY || o.ch == EMPTY {
                continue;
            }
            let mask_o = (o.ch as u32) - 0x2800;
            let mask_b = (b.ch as u32) - 0x2800;
            assert_eq!(
                mask_b, mask_o,
                "combined cell {i} must equal overlay-only mask (always-replace), \
                 got {mask_b:08b}; overlay mask was {mask_o:08b}"
            );
            assert_eq!(b.fg, 11, "combined cell {i} fg must be the overlay colour");
            overlap_count += 1;
        }
        assert!(
            overlap_count > 0,
            "no overlay-on-tile-feature crossing detected — adjust the \
             overlay's coords or zoom so the line passes through a road cell"
        );
    }
}
