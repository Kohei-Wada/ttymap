use clap::Parser;
use ttymap::app::frame_timer::FrameTimer;
use ttymap::app::{App, KeybindingOverrides};
use ttymap::cli::Command as Subcommand;
use ttymap::config::Config;
use ttymap::input::thread::InputHandle;

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

    /// Write debug logs to ~/.local/state/ttymap/ttymap.log. Optional
    /// level argument: `--log` alone is `debug`; `--log info` /
    /// `--log trace` etc. select an explicit level. Without the flag
    /// no logger is installed (log macros become no-ops).
    #[arg(long, value_name = "LEVEL", num_args = 0..=1, default_missing_value = "debug")]
    log: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Logging is opt-in via `--log [LEVEL]`. Without the flag no
    // logger is registered and the `log::*!` macros are no-ops.
    // When set, logs land in ~/.local/state/ttymap/ttymap.log
    // (truncated on each launch, so debug sessions don't accumulate).
    if let Some(level) = cli.log.as_deref() {
        ttymap::logging::init(level).ok();
    }

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
    // interactive app.
    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.run() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Run init.lua first, then override with CLI args. `keymap_overrides`
    // travels to App::new alongside Config because the keymap is
    // scripted at the same place but lives in its own data shape.
    let (mut config, keymap_overrides) = ttymap::lua::load_init_lua(Config::default());

    if let Some(v) = cli.lat {
        config.engine.map.lat = v;
    }
    if let Some(v) = cli.lon {
        config.engine.map.lon = v;
    }
    if let Some(v) = cli.zoom {
        config.engine.map.zoom = Some(v);
    }
    if let Some(v) = cli.style {
        // Unknown values get normalised to "dark" by the styler's
        // fallback at construction time; just hand the raw string in.
        config.engine.render.style = v;
    }

    if cli.here || config.geoip.on_startup {
        match ttymap::shared::geoip::lookup(&config.geoip.endpoint, config.geoip.timeout_ms) {
            Some((lat, lon)) => {
                log::info!("geoip: resolved to {}, {}", lat, lon);
                config.engine.map.lat = lat;
                config.engine.map.lon = lon;
            }
            None => {
                log::warn!(
                    "geoip lookup failed, using default {}, {}",
                    config.engine.map.lat,
                    config.engine.map.lon
                );
            }
        }
    }

    log::info!(
        "starting ttymap: lat={}, lon={}",
        config.engine.map.lat,
        config.engine.map.lon
    );

    if let Err(e) = run_event_loop(config, keymap_overrides) {
        eprintln!("Error: {e}");
    }
}

/// Composition root: builds the event channel, every subsystem
/// (map / Lua), spawns the off-thread input / frame-timer peers,
/// then hands control to `App::run`. Every thread handle joins
/// in its `Drop` impl, so teardown is just RAII at end of scope.
fn run_event_loop(config: Config, keymap_overrides: KeybindingOverrides) -> std::io::Result<()> {
    let (event_tx, event_rx) = std::sync::mpsc::channel();

    // Active theme — owned by App, consumed by the map only at
    // construction (initial styler) and on theme switch.
    let theme_id = ttymap::theme::ThemeId::from_name(&config.engine.render.style);

    // Engine doesn't depend on crossterm; the binary owns the
    // terminal-size probe and hands cols/rows to `engine::map::build`.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    // Frame sink — the engine doesn't know about `AppEvent`. We hand
    // it a closure that wraps each completed `MapFrame` into the
    // binary's bus protocol. Returning `false` tells the engine the
    // bus is closed and the render thread should exit.
    let frame_tx = event_tx.clone();
    let frame_sink: ttymap_engine::map::render::thread::FrameSink = Box::new(move |frame| {
        frame_tx
            .send(ttymap::app::AppEvent::FrameReady(frame))
            .is_ok()
    });

    // Map subsystem: tile cache + render pipeline + render thread.
    // `_render_handle` is a peer to `_input` / `_frame_timer` — held
    // here for `Drop`-driven shutdown, not used otherwise.
    let (_render_handle, map) =
        ttymap_engine::map::build(&config.engine, cols, rows, frame_sink, theme_id);

    // Keymap is shared input by both the Lua subsystem (help plugin
    // displays it; palette uses it for prefix matching) and the
    // compositor's BaseLayer at runtime — build it once here.
    let keymap = ttymap::input::KeyMap::with_overrides(&keymap_overrides);

    // Lua subsystem: load every plugin, register activations / palette
    // entries / event-bus subscriptions, return the populated bundle.
    // All Lua → App traffic rides the shared `OpsBuffer` built
    // inside `build_subsystem`; no separate intent sender needed.
    let mut lua = ttymap::lua::build_subsystem(&config, map.attribution.clone(), &keymap);

    // Palette is a built-in (not a plugin): drain every plugin's
    // palette_entries into a CommandSeed and append the `:` activation.
    // Must run after every plugin's register call.
    ttymap::palette::install(
        &keymap,
        &mut lua.activations,
        std::mem::take(&mut lua.palette_entries),
    );

    let mut app = App::new(config, keymap, theme_id, map, lua);

    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    // Push the kitty keyboard protocol's DISAMBIGUATE flag so
    // C-j arrives as `Char('j') + CONTROL` instead of being
    // collapsed onto `Enter` (= ASCII LF = legacy C-j). Required
    // for the C-j / C-k focus-cycle keybind to be distinct from
    // Enter (palette submit, plugin "jump"). If the terminal
    // doesn't speak the protocol the push is a no-op; we ignore
    // the error so non-supporting terminals still boot.
    let kitty_pushed = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::PushKeyboardEnhancementFlags(
            crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        )
    )
    .is_ok();

    let _input = InputHandle::spawn(event_tx.clone(), app.poll_timeout());
    let _frame_timer = FrameTimer::spawn(event_tx.clone(), app.poll_timeout());

    log::info!("event loop started");
    app.run(&mut terminal, &event_rx, &event_tx)?;
    log::info!("event loop ended");

    if kitty_pushed {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags
        );
    }
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)?;
    ratatui::restore();
    log::info!("terminal restored, exiting");
    Ok(())
}
