//! Map subsystem — domain state and the full map-rendering pipeline.
//!
//! `state.rs` / `action.rs` own the map viewport (center, zoom, running
//! flag) and the `MapAction` enum. The siblings are the implementation
//! machinery:
//!
//! - `tile/`    — MVT fetch + cache + decode
//! - `styler/`  — GL-style rules (dark / bright presets)
//! - `render/`  — render thread + pipeline + drawing primitives
//!
//! Everything map-specific lives under this module; the UI consumes
//! `MapFrame` (from `render`) without knowing how it was produced.

pub mod action;
pub mod render;
pub mod state;
pub mod styler;
pub mod tile;

pub use action::MapAction;
pub use state::{MapState, MapStateOptions, Viewport};

use std::sync::Arc;

use crate::config::Config;
use crate::geo::LonLat;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::{FrameSink, RenderClient, RenderHandle};
use crate::map::styler::Styler;
use crate::theme::ThemeId;

/// Runtime handle to the map subsystem.
///
/// App stores one of these and interacts with the map only
/// through the methods below — never names `MapState`, the render
/// channel, the styler, or the underlying viewport machinery.
///
/// The owning [`RenderHandle`] is **not** in here — it lives in
/// `main`'s scope so its `Drop` (Shutdown + join) fires at the
/// composition root, peer to `InputHandle` / `FrameTimer`.
///
/// The active `ThemeId` is **not** stored here either — the theme is
/// an app-level concern owned by `App`. The map only consumes
/// it transiently to build a `Styler` when [`Self::set_theme`] is
/// called.
pub struct MapHandle {
    state: MapState,
    render_client: RenderClient,
    /// Tile-source attribution string. The Lua subsystem reads this
    /// once at register time (passed to plugin shared state); main
    /// is the sole reader after construction.
    pub attribution: Option<String>,
}

impl MapHandle {
    // ── Queries ────────────────────────────────────────────────────────

    /// Active centre in lon/lat — what every Lua plugin's
    /// `ttymap.map:center()` mirror cell tracks.
    pub fn center(&self) -> LonLat {
        self.state.center()
    }

    /// Active zoom level.
    pub fn zoom(&self) -> f64 {
        self.state.zoom()
    }

    // ── Mutations ──────────────────────────────────────────────────────

    /// Apply a map-level [`MapAction`] (pan / zoom / jump / reset / …).
    /// Returns `true` if the state changed in a way that warrants a
    /// redraw.
    pub fn apply_action(&mut self, action: &MapAction) -> bool {
        self.state.process_action(action)
    }

    /// Resize the canvas — both the in-process [`MapState`] (so the
    /// next viewport is computed for the new dimensions) and the
    /// render thread's pipeline (so it allocates a new
    /// canvas-sized buffer).
    pub fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.state.resize(cols, rows);
        self.render_client
            .request_resize(self.state.width(), self.state.height());
    }

    /// Switch the active theme on the render thread: build a fresh
    /// styler from `new_id` and ship it. The theme id itself is owned
    /// by the caller (App); the map only needs it transiently
    /// to construct a [`Styler`].
    pub fn set_theme(&self, new_id: ThemeId) {
        let styler = Arc::new(Styler::new(new_id));
        self.render_client.set_styler(styler);
    }

    /// Toggle tile-rendered text labels on the render thread.
    /// Caller is responsible for the follow-up [`Self::request_redraw`]
    /// — flipping the flag alone won't redraw the visible frame.
    pub fn set_labels_visible(&self, visible: bool) {
        self.render_client.set_labels_visible(visible);
    }

    /// Show / hide one MVT source layer on the render thread.
    /// Caller is responsible for the follow-up [`Self::request_redraw`]
    /// — updating the hidden set alone won't redraw the visible frame.
    pub fn set_layer_visible(&self, layer: &str, visible: bool) {
        self.render_client.set_layer_visible(layer, visible);
    }

    /// Queue a fresh `RenderTask::Draw` against the current
    /// viewport, carrying any per-frame overlays the caller has
    /// collected (e.g. Lua-pushed polylines drained from
    /// `overlay_sink`).
    pub fn request_redraw(&self, overlays: Vec<render::overlay::UserPolyline>) {
        self.render_client
            .request_draw(self.state.viewport(), overlays);
    }
}

/// Build the map subsystem: tile cache + render pipeline + render
/// thread + initial `MapState`. Returns `(RenderHandle, MapHandle)` —
/// main holds the owning thread guard for `Drop`-driven shutdown,
/// and hands the handle to `App::new`.
///
/// `theme_id` is consumed transiently to build the initial styler;
/// the map subsystem doesn't keep it. The active theme lives on
/// `App` since it crosses both map rendering and UI chrome.
pub fn build(
    config: &Config,
    cache_dir: Option<&std::path::Path>,
    cols: u16,
    rows: u16,
    frame_sink: FrameSink,
    theme_id: ThemeId,
) -> Result<(RenderHandle, MapHandle), crate::EngineError> {
    let (width, height) = render::canvas_size(cols, rows);

    log::info!(
        "terminal size: {}x{}, canvas: {}x{}",
        cols,
        rows,
        width,
        height
    );

    let (tile_cache, wake_rx) = tile::build(config, cache_dir)?;
    let attribution = tile_cache.attribution();

    let styler = Arc::new(Styler::new(theme_id));

    let pipeline = RenderPipeline::new(
        tile_cache,
        styler,
        config.render.language.clone(),
        width,
        height,
    );
    let render_handle = RenderHandle::spawn(pipeline, wake_rx, frame_sink);
    let render_client = render_handle.client();

    // Engine has no built-in viewport opinion: if the binary
    // didn't seed `config.map.lat/lon`, fall back to (0,0) so we
    // still produce a frame. The binary is responsible for picking
    // a meaningful starting view (CLI flag / init.lua / app
    // default — see `ttymap-app` and `ttymap-cli`).
    let state = MapState::new(
        MapStateOptions {
            initial_lon: config.map.lon.unwrap_or(0.0),
            initial_lat: config.map.lat.unwrap_or(0.0),
            initial_zoom: config.map.zoom,
            zoom_step: config.map.zoom_step,
            max_zoom: config.map.max_zoom,
        },
        width,
        height,
    );

    Ok((
        render_handle,
        MapHandle {
            state,
            render_client,
            attribution,
        },
    ))
}
