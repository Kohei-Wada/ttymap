use clap::Parser;
use ttymap::commands::Command as Subcommand;
use ttymap::config::Config;
use ttymap::frontend::frame_timer::FrameTimer;
use ttymap::frontend::input_thread::InputHandle;
use ttymap::frontend::{Frontend, KeybindingOverrides};

#[derive(Parser)]
#[command(
    name = "ttymap",
    about = "Terminal map viewer — renders Mapbox Vector Tiles as Braille characters",
    long_about = "ttymap is a terminal-based map viewer written in Rust.\n\
        It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters\n\
        with ANSI 256-color in your terminal.\n\n\
        Inspired by and based on mapscii (https://github.com/rastapasta/mapscii).\n\n\
        Config file:    ~/.config/ttymap/init.lua\n\
        User plugins:   ~/.config/ttymap/plugins/\n\
        Bundled runtime: ~/.local/share/ttymap/lua/\n\
        Log file:       ~/.local/state/ttymap/ttymap.log\n\
        Tile cache:     ~/.cache/ttymap/",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Subcommand>,

    /// Initial latitude
    #[arg(long, conflicts_with = "here")]
    lat: Option<f64>,

    /// Initial longitude
    #[arg(long, conflicts_with = "here")]
    lon: Option<f64>,

    /// Initial zoom level
    #[arg(long, short)]
    zoom: Option<f64>,

    /// Style preset (dark, bright)
    #[arg(long)]
    style: Option<String>,

    /// Jump to IP-based current location on startup
    #[arg(long)]
    here: bool,
}

fn main() {
    let cli = Cli::parse();

    // Logging initialised up-front so both subcommands and the
    // interactive app write to ~/.local/state/ttymap/ttymap.log.
    ttymap::logging::init().ok();

    // Resolve runtime path before any Lua state spins up. Both the
    // interactive app and the `snap` subcommand reach for it; doing
    // this once at the top means we fail fast with a single error
    // message rather than hitting the same wall in two places.
    let runtime_path = match ttymap::lua::resolve_runtime_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ttymap: {}", e);
            std::process::exit(1);
        }
    };
    ttymap::lua::set_runtime_path(runtime_path);

    // Subcommands run a single task and exit without booting the full
    // interactive frontend.
    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.run() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Run init.lua first, then override with CLI args. `keymap_overrides`
    // travels to Frontend::new alongside Config because the keymap is
    // scripted at the same place but lives in its own data shape.
    let (mut config, keymap_overrides) = ttymap::lua::run_init_lua(Config::default());

    if let Some(v) = cli.lat {
        config.map.lat = v;
    }
    if let Some(v) = cli.lon {
        config.map.lon = v;
    }
    if let Some(v) = cli.zoom {
        config.map.zoom = Some(v);
    }
    if let Some(v) = cli.style {
        // Unknown values get normalised to "dark" by the styler's
        // fallback at construction time; just hand the raw string in.
        config.render.style = v;
    }

    if cli.here || config.geoip.on_startup {
        match ttymap::shared::geoip::lookup(&config.geoip.endpoint, config.geoip.timeout_ms) {
            Some((lat, lon)) => {
                log::info!("geoip: resolved to {}, {}", lat, lon);
                config.map.lat = lat;
                config.map.lon = lon;
            }
            None => {
                log::warn!(
                    "geoip lookup failed, using default {}, {}",
                    config.map.lat,
                    config.map.lon
                );
            }
        }
    }

    log::info!(
        "starting ttymap: lat={}, lon={}",
        config.map.lat,
        config.map.lon
    );

    if let Err(e) = run_event_loop(config, keymap_overrides) {
        eprintln!("Error: {e}");
    }
}

/// The composition root: builds the event channel and the Lua bus,
/// constructs `Frontend` + every off-thread subsystem (render /
/// input / frame timer), then drives the per-iteration loop.
/// `Frontend` is just a state-mutating handler invoked by the loop;
/// the bus, channels, and threads are peer participants on the same
/// bus, all wired up here in `main` rather than implicitly inside
/// the frontend.
fn run_event_loop(config: Config, keymap_overrides: KeybindingOverrides) -> std::io::Result<()> {
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (mut frontend, render_handle, event_bus) =
        Frontend::new(config, keymap_overrides, event_tx.clone());

    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;

    // Subsystems are peers to the Frontend on the same bus — main spawns
    // each with its own `event_tx` clone. `Drop` order here matters
    // for clean teardown: input thread / frame timer first (they
    // read from a still-live receiver), then `render_handle` after
    // the loop has stopped consuming frames.
    let _input = InputHandle::spawn(event_tx.clone(), frontend.poll_timeout());
    let _frame_timer = FrameTimer::spawn(event_tx.clone(), frontend.poll_timeout());

    log::info!("event loop started");
    frontend.dispatch_initial_redraw();

    while frontend.is_running() {
        // Per-plugin housekeeping before the event drain — Lua plugins
        // queue components via `ttymap.api.window.open` here and the
        // current iteration's `poll_compositor` already sees them.
        frontend.refresh_lua_host_state_per_tick();

        // Park on the unified bus until any source produces an
        // event; drain any further buffered events non-blockingly
        // so a burst doesn't push the paint behind.
        match event_rx.recv() {
            Ok(event) => frontend.handle_event(event, &event_bus, &event_tx),
            Err(_) => break,
        }
        while let Ok(event) = event_rx.try_recv() {
            frontend.handle_event(event, &event_bus, &event_tx);
        }

        // Component poll: any `win.emit(msg)` inside fires onto the
        // bus directly. Same-iteration `try_recv` ran above already;
        // an emission here will be picked up next iteration.
        frontend.poll_compositor(&event_tx);

        // Render a frame. Inside `ui::draw`, the per-frame Lua
        // `tick` event fires against the live MapApi.
        frontend.render_into(&mut terminal, &event_bus)?;

        // If plugin `on_tick` callbacks pushed polylines, throttle
        // the redraw request to the configured interval.
        frontend.tick_overlay_redraw();
    }

    log::info!("event loop ended");
    drop(_input);
    drop(_frame_timer);
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)?;
    ratatui::restore();
    log::info!("terminal restored, exiting");
    drop(render_handle);
    Ok(())
}
