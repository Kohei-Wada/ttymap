//! Render pipeline — owns tile cache and renderer.
//! Encapsulates the full data flow: RenderRequest → tile fetch → draw → frame string.
//! thread.rs calls pipeline methods without knowing the internals.

use std::sync::Arc;

use rstar::AABB;

use super::frame::MapFrame;
use super::renderer::{Renderer, TileData};
use super::view::{VisibleTile, visible_tiles};
use crate::map::RenderRequest;
use crate::styler::Styler;
use crate::tile::{Feature, TileCache};

pub struct RenderPipeline {
    tile_cache: TileCache,
    renderer: Renderer,
}

impl RenderPipeline {
    /// Build a pipeline from its two owned subsystems. The caller
    /// constructs the `TileCache` (see `app::build_tile_cache`) so
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

    /// Process a RenderRequest into a MapFrame.
    /// Returns None if no tiles are available yet.
    pub fn render(&mut self, state: &RenderRequest) -> Option<MapFrame> {
        let z = crate::geo::base_zoom(state.zoom);
        self.tile_cache
            .set_view(state.center.lon, state.center.lat, z);
        let visible = visible_tiles(
            state.center.lon,
            state.center.lat,
            state.zoom,
            self.renderer.width(),
            self.renderer.height(),
        );
        let tile_data = self.collect_tile_data(&visible, state.zoom);
        self.renderer.draw(&tile_data, state.zoom).map(|mut f| {
            f.center = state.center;
            f.zoom = state.zoom;
            f
        })
    }

    /// Poll for completed tile fetches. Returns true if new tiles arrived.
    pub fn poll_tiles(&mut self) -> bool {
        self.tile_cache.poll_completed()
    }

    /// Prefetch surrounding tiles (call when idle).
    pub fn prefetch(&mut self, state: &RenderRequest) {
        self.tile_cache
            .prefetch(state.center.lon, state.center.lat, state.zoom);
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
            let mut layers: Vec<(String, Vec<Feature>)> = Vec::new();

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
                        layers.push((name, features));
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
