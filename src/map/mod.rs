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
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::{RenderClient, RenderHandle};
use crate::map::styler::Styler;
use crate::theme::ThemeId;

/// Initialised map subsystem handed to `Frontend::new`.
///
/// `RenderHandle` is **not** in here — it is the owning thread guard
/// and lives in `main`'s scope so its `Drop` (Shutdown + join) fires
/// at the composition root, peer to `InputHandle` / `FrameTimer`. The
/// fields below are everything Frontend needs to know about the map
/// once construction is done: a cheap-clone task sender, the data
/// the Lua subsystem reads at register time (attribution), the
/// canvas dimensions used by `MapState` and the render pipeline, and
/// the active theme id.
pub struct MapHandle {
    pub render_client: RenderClient,
    pub attribution: Option<String>,
    pub width: usize,
    pub height: usize,
    pub theme_id: ThemeId,
}

/// Build the map subsystem: tile cache + render pipeline + render
/// thread. Returns `(RenderHandle, MapHandle)` — main holds the
/// owning handle for `Drop`-driven shutdown, and hands the rest to
/// `Frontend::new`.
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

    (
        render_handle,
        MapHandle {
            render_client,
            attribution,
            width,
            height,
            theme_id,
        },
    )
}
