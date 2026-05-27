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
    render_client: RenderClient,
    /// Tile-source attribution string. The Lua subsystem reads this
    /// once at register time (passed to plugin shared state); main
    /// is the sole reader after construction.
    pub attribution: Option<String>,
}

impl MapHandle {
    // ── Mutations ──────────────────────────────────────────────────────

    /// Reallocate the render thread's canvas buffer for the new
    /// terminal size. The post-resize viewport reaches the engine via
    /// the following `request_draw` — `MapHandle` holds no camera
    /// state, so it does not recompute a centre/zoom here.
    pub fn resize(&self, cols: u16, rows: u16) {
        let (width, height) = render::canvas_size(cols, rows);
        self.render_client.request_resize(width, height);
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

    /// Queue a fresh `RenderTask::Draw` at the supplied `viewport`,
    /// carrying any per-frame overlays the caller collected (e.g.
    /// Lua-pushed polylines drained from `overlay_sink`). The
    /// viewport is computed by the App — the engine holds no camera
    /// state of its own.
    pub fn request_draw(&self, viewport: Viewport, overlays: Vec<render::overlay::UserPolyline>) {
        self.render_client.request_draw(viewport, overlays);
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

    Ok((
        render_handle,
        MapHandle {
            render_client,
            attribution,
        },
    ))
}
