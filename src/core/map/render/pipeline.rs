//! Render pipeline — owns tile cache and renderer.
//! Encapsulates the full data flow: Viewport → tile fetch → draw → MapFrame.
//! thread.rs calls pipeline methods without knowing the internals.

use std::sync::Arc;

use rstar::AABB;

use super::frame::MapFrame;
use super::renderer::{LayerData, Renderer, TileData};
use super::view::{VisibleTile, visible_tiles};
use crate::core::map::Viewport;
use crate::core::map::styler::Styler;
use crate::core::map::tile::{Feature, TileCache};

pub struct RenderPipeline {
    tile_cache: TileCache,
    renderer: Renderer,
}

impl RenderPipeline {
    /// Build a pipeline from its two owned subsystems. The caller
    /// constructs the `TileCache` (see [`crate::core::map::tile::build`]) so
    /// the backend selection stays visible at the composition root.
    pub fn new(
        tile_cache: TileCache,
        styler: Arc<Styler>,
        language: String,
        width: usize,
        height: usize,
    ) -> Self {
        let renderer = Renderer::new(styler, language, width, height);
        Self {
            tile_cache,
            renderer,
        }
    }

    /// Process a `Viewport` into a `MapFrame`.
    /// Returns `None` if no tiles are available yet.
    pub fn render(
        &mut self,
        vp: &Viewport,
        overlays: &[crate::core::map::render::overlay::UserPolyline],
    ) -> Option<MapFrame> {
        let z = crate::geo::base_zoom(vp.zoom);
        self.tile_cache.set_view(vp.center.lon, vp.center.lat, z);
        let visible = self.visible_tiles_for(vp);
        let tile_data = self.collect_tile_data(&visible, vp.zoom);
        self.renderer
            .draw(&tile_data, vp.zoom, vp.center, overlays)
            .map(|mut f| {
                f.center = vp.center;
                f.zoom = vp.zoom;
                f
            })
    }

    /// Poll for completed tile fetches. Returns true if new tiles arrived.
    pub fn poll_tiles(&mut self) -> bool {
        self.tile_cache.poll_completed()
    }

    /// Whether the tile backend has finished all outstanding fetches.
    /// See [`crate::core::map::tile::cache::TileCache::is_fetch_idle`].
    pub fn is_tile_fetch_idle(&self) -> bool {
        self.tile_cache.is_fetch_idle()
    }

    /// Prefetch tiles around the current view so pan / zoom feels
    /// instant instead of flashing black while HTTP fetches run.
    ///
    /// Three layers:
    /// 1. Pan ring at the current zoom (inherited from
    ///    `tile_cache::prefetch` — a 2-tile ring around the center).
    /// 2. **Every visible tile's four children at z+1**, so a
    ///    zoom-in lands on already-warm tiles wherever the user is
    ///    looking (not just near the center).
    /// 3. **Every visible tile's parent at z-1** (deduped because
    ///    adjacent tiles share a parent), so zoom-out is also warm.
    pub fn prefetch(&mut self, vp: &Viewport) {
        // Current-zoom pan ring + center-tile z±1 (kept as-is).
        self.tile_cache
            .prefetch(vp.center.lon, vp.center.lat, vp.zoom);

        let visible = self.visible_tiles_for(vp);

        let z = crate::geo::base_zoom(vp.zoom);

        // z+1: every visible tile's four children.
        if z < 14 {
            let child_z = z + 1;
            let child_grid = crate::geo::tile_grid_size(child_z);
            for vt in &visible {
                let bx = vt.x * 2;
                let by = vt.y * 2;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let ty = by + dy;
                        if ty < 0 || ty >= child_grid {
                            continue;
                        }
                        let tx = (bx + dx).rem_euclid(child_grid);
                        self.tile_cache.get_tile(child_z, tx, ty);
                    }
                }
            }
        }

        // z-1: parent of every visible tile (adjacent tiles share
        // parents, so dedupe).
        if z > 0 {
            let parent_z = z - 1;
            let parent_grid = crate::geo::tile_grid_size(parent_z);
            let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
            for vt in &visible {
                let py = vt.y / 2;
                let px = vt.x.div_euclid(2).rem_euclid(parent_grid);
                if py < 0 || py >= parent_grid {
                    continue;
                }
                if seen.insert((px, py)) {
                    self.tile_cache.get_tile(parent_z, px, py);
                }
            }
        }
    }

    /// Resize the renderer canvas.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.renderer.set_size(width, height);
    }

    /// Swap the active `Styler` — used for runtime theme switching.
    /// Cached decoded tiles are theme-agnostic (`tile::decode` does not
    /// consult a styler), so no cache flush is needed.
    pub fn set_styler(&mut self, styler: Arc<Styler>) {
        self.renderer.set_styler(styler);
    }

    // ── Private ──────────────────────────────────────────────────────────

    /// Compute the set of visible tiles for a viewport against the
    /// renderer's current canvas. Centralised so `render` and
    /// `prefetch` share the same call shape (both consume the same
    /// function of the same inputs, just in different iterations of
    /// the render-thread loop).
    fn visible_tiles_for(&self, vp: &Viewport) -> Vec<VisibleTile> {
        visible_tiles(
            vp.center.lon,
            vp.center.lat,
            vp.zoom,
            self.renderer.width(),
            self.renderer.height(),
        )
    }

    /// Collect features from tile cache for visible tiles.
    /// Bridge between tile subsystem and renderer.
    fn collect_tile_data(&mut self, visible: &[VisibleTile], zoom: f64) -> Vec<TileData> {
        let draw_order = Renderer::draw_order(zoom);
        let width = self.renderer.width();
        let height = self.renderer.height();
        let mut result = Vec::new();

        for vis in visible {
            let decoded = match self.tile_cache.get_tile(vis.z, vis.x, vis.y) {
                Some(t) => t,
                None => {
                    result.push(TileData {
                        vis: vis.clone(),
                        layers: Vec::new(),
                    });
                    continue;
                }
            };

            let tile_size = vis.size;
            let mut layers: Vec<LayerData> = Vec::new();

            for layer_name in &draw_order {
                let name = layer_name.to_string();
                if let Some(tile_layer) = decoded.layers.get(&name) {
                    let extent = tile_layer.extent as f64;
                    let scale = extent / tile_size;
                    let envelope = AABB::from_corners(
                        [-vis.pos_x * scale, -vis.pos_y * scale],
                        [
                            (width as f64 - vis.pos_x) * scale,
                            (height as f64 - vis.pos_y) * scale,
                        ],
                    );
                    let features: Vec<Feature> = tile_layer
                        .tree
                        .locate_in_envelope_intersecting(&envelope)
                        .cloned()
                        .collect();
                    if !features.is_empty() {
                        layers.push(LayerData {
                            name,
                            extent: tile_layer.extent,
                            features,
                        });
                    }
                }
            }

            result.push(TileData {
                vis: vis.clone(),
                layers,
            });
        }

        result
    }
}
