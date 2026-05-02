//! Map subsystem — domain state and the full map-rendering pipeline.
//!
//! `state.rs` / `action.rs` own the map viewport (center, zoom, running
//! flag) and the `Action` enum. The siblings are the implementation
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

pub use action::Action;
pub use state::{MapState, MapStateOptions, Viewport};

use std::sync::Arc;
use std::sync::mpsc;

use crate::config::Config;
use crate::frontend::AppEvent;
use crate::geo::LonLat;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::{RenderClient, RenderHandle};
use crate::map::styler::Styler;
use crate::theme::ThemeId;

/// Runtime handle to the map subsystem.
///
/// Frontend stores one of these and interacts with the map only
/// through the methods below — never names `MapState`, the render
/// channel, the styler, or the underlying viewport machinery.
///
/// The owning [`RenderHandle`] is **not** in here — it lives in
/// `main`'s scope so its `Drop` (Shutdown + join) fires at the
/// composition root, peer to `InputHandle` / `FrameTimer`.
pub struct MapHandle {
    state: MapState,
    render_client: RenderClient,
    /// Tile-source attribution string. The Lua subsystem reads this
    /// once at register time (passed to plugin shared state); main
    /// is the sole reader after construction.
    pub attribution: Option<String>,
    theme_id: ThemeId,
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

    /// Active theme id. Read by Frontend to seed the per-frame
    /// `Context` handed to components (`Component::handle_event` /
    /// `Component::render` / `Component::paint_on_map`).
    pub fn theme_id(&self) -> ThemeId {
        self.theme_id
    }

    // ── Mutations ──────────────────────────────────────────────────────

    /// Apply a map-level [`Action`] (pan / zoom / jump / reset / …).
    /// Returns `true` if the state changed in a way that warrants a
    /// redraw.
    pub fn apply_action(&mut self, action: &Action) -> bool {
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

    /// Switch the active theme: update the stored `ThemeId`, build
    /// a fresh styler, and ship it to the render thread.
    pub fn set_theme(&mut self, new_id: ThemeId) {
        self.theme_id = new_id;
        let styler = Arc::new(Styler::new(new_id));
        self.render_client.set_styler(styler);
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
/// and hands the handle to `Frontend::new`.
pub fn build(config: &Config, event_tx: mpsc::Sender<AppEvent>) -> (RenderHandle, MapHandle) {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (width, height) = render::canvas_size(cols, rows);

    log::info!(
        "terminal size: {}x{}, canvas: {}x{}",
        cols,
        rows,
        width,
        height
    );

    let (tile_cache, wake_rx) = tile::build(config);
    let attribution = tile_cache.attribution();

    let theme_id = ThemeId::from_name(&config.render.style);
    let styler = Arc::new(Styler::new(theme_id));

    let pipeline = RenderPipeline::new(
        tile_cache,
        styler,
        config.render.language.clone(),
        width,
        height,
    );
    let render_handle = RenderHandle::spawn(pipeline, wake_rx, event_tx);
    let render_client = render_handle.client();

    let state = MapState::new(
        MapStateOptions {
            initial_lon: config.map.lon,
            initial_lat: config.map.lat,
            initial_zoom: config.map.zoom,
            zoom_step: config.map.zoom_step,
            max_zoom: config.map.max_zoom,
        },
        width,
        height,
    );

    (
        render_handle,
        MapHandle {
            state,
            render_client,
            attribution,
            theme_id,
        },
    )
}
