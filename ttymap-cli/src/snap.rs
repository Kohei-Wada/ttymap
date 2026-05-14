//! `ttymap snap` — headless single-frame renderer.
//!
//! Builds the same render pipeline the interactive app uses but runs
//! it **synchronously** on the main thread: no render thread, no
//! mpsc channels. A simple fetch-aware loop drives `pipeline.render`
//! and `pipeline.poll_tiles` until all visible tiles have arrived (or
//! the timeout hits), then serialises the final frame through
//! [`MapFrame::to_ansi`] and writes to stdout or a file.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use clap::Args;

use ttymap_core::config;
use ttymap_engine::map::Viewport;
use ttymap_engine::map::render::frame::MapFrame;
use ttymap_engine::map::render::pipeline::RenderPipeline;
use ttymap_engine::map::styler::Styler;
use ttymap_engine::map::{MapState, MapStateOptions};
use ttymap_engine::theme::ThemeId;

/// Polling step between tile checks. Short enough to feel responsive,
/// long enough to avoid busy-waiting the CPU while HTTP fetches run
/// on the tile-worker threads.
const POLL_STEP: Duration = Duration::from_millis(50);

#[derive(Args)]
pub struct SnapArgs {
    /// Center latitude.
    #[arg(long)]
    pub lat: Option<f64>,

    /// Center longitude.
    #[arg(long)]
    pub lon: Option<f64>,

    /// Zoom level (1–18, roughly). If omitted, uses the config /
    /// auto-zoom default.
    #[arg(long, short)]
    pub zoom: Option<f64>,

    /// Output width in terminal cells. Defaults to the current
    /// terminal width (80 if no TTY is attached).
    #[arg(long)]
    pub cols: Option<u16>,

    /// Output height in terminal cells. Defaults to the current
    /// terminal height (24 if no TTY is attached).
    #[arg(long)]
    pub rows: Option<u16>,

    /// Style preset (dark, bright). Defaults to the config value.
    #[arg(long)]
    pub style: Option<String>,

    /// Label language (e.g. "en", "ja"). Defaults to the config value.
    #[arg(long)]
    pub language: Option<String>,

    /// Write the ANSI output to this file instead of stdout.
    #[arg(long, short)]
    pub output: Option<PathBuf>,

    /// Give up after this many milliseconds if tiles never finish
    /// loading.
    #[arg(long, default_value_t = 10_000)]
    pub timeout_ms: u64,
}

pub fn run(args: SnapArgs) -> Result<(), Box<dyn std::error::Error>> {
    // snap is headless and doesn't activate plugins, so we use the
    // config-only init.lua entry (no API install, no plugin requires).
    // `Config` carries every init.lua-tunable knob (cache / render);
    // plugins would just slow snap down.
    let mut config = ttymap_lua::read_init_lua_config_only(config::Config::default());

    if let Some(lat) = args.lat {
        config.engine.map.lat = lat;
    }
    if let Some(lon) = args.lon {
        config.engine.map.lon = lon;
    }
    if let Some(z) = args.zoom {
        config.engine.map.zoom = Some(z);
    }
    if let Some(s) = args.style {
        config.engine.render.style = s;
    }
    if let Some(lang) = args.language {
        config.engine.render.language = lang;
    }

    // If --cols/--rows weren't given, default to the current
    // terminal size. Falls back to 80×24 when stdout isn't a TTY
    // (e.g. redirected to a file or run under `cron`).
    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let cols = args.cols.unwrap_or(term_cols);
    let rows = args.rows.unwrap_or(term_rows);

    // Snap wants the whole output to be the map — no border / footer
    // subtracted. 1 cell = 2×4 Braille sub-pixels.
    let width = cols as usize * 2;
    let height = rows as usize * 4;

    // tile::build spawns 6 worker threads fetching tiles in
    // parallel — they run independently of us, so we can drive the
    // pipeline synchronously and just poll for completed tiles.
    let (tile_cache, _wake_rx) = ttymap_engine::map::tile::build(&config.engine)?;
    let theme_id = ThemeId::from_name(&config.engine.render.style);
    let styler = Arc::new(Styler::new(theme_id));
    let mut pipeline = RenderPipeline::new(
        tile_cache,
        styler,
        config.engine.render.language.clone(),
        width,
        height,
    );

    let map = MapState::new(
        MapStateOptions {
            initial_lon: config.engine.map.lon,
            initial_lat: config.engine.map.lat,
            initial_zoom: config.engine.map.zoom,
            zoom_step: config.engine.map.zoom_step,
            max_zoom: config.engine.map.max_zoom,
        },
        width,
        height,
    );

    let frame = wait_for_stable_frame(
        &mut pipeline,
        &map.viewport(),
        Duration::from_millis(args.timeout_ms),
    )?;
    let ansi = frame.to_ansi();

    match args.output {
        Some(path) => fs::write(&path, ansi)?,
        None => io::stdout().write_all(ansi.as_bytes())?,
    }
    Ok(())
}

/// Drive the pipeline until all visible tiles have been fetched and
/// drawn, or `timeout` elapses.
///
/// The first `pipeline.render` call queues up tile fetches as a side
/// effect of `tile_cache::set_view`; subsequent iterations poll for
/// completed tiles and redraw when any arrive. We stop as soon as
/// `pipeline.is_tile_fetch_idle()` reports the backend has drained
/// both its queue and its in-flight set — at that point no further
/// tile arrivals are possible without another request.
fn wait_for_stable_frame(
    pipeline: &mut RenderPipeline,
    viewport: &Viewport,
    timeout: Duration,
) -> io::Result<MapFrame> {
    let deadline = Instant::now() + timeout;

    // First render: probably empty (no tiles yet) but kicks off the
    // tile fetches by calling `tile_cache::set_view` internally.
    let mut last_frame = pipeline.render(viewport, &[]);

    loop {
        if Instant::now() >= deadline {
            break;
        }

        if pipeline.poll_tiles()
            && let Some(f) = pipeline.render(viewport, &[])
        {
            last_frame = Some(f);
        }

        if pipeline.is_tile_fetch_idle() {
            // Drain any tiles that landed between our last poll_tiles
            // and the idle check, then finalise.
            if pipeline.poll_tiles()
                && let Some(f) = pipeline.render(viewport, &[])
            {
                last_frame = Some(f);
            }
            break;
        }

        thread::sleep(POLL_STEP);
    }

    last_frame.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "no tiles loaded within {:?}; check network or raise --timeout-ms",
                timeout
            ),
        )
    })
}
