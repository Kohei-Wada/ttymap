use clap::Parser;
use ttymap_app::app::App;
use ttymap_app::app::frame_timer::FrameTimer;
use ttymap_cli::Command as Subcommand;
use ttymap_config::{AppDirs, Config};
use ttymap_tui::input::thread::InputHandle;

#[derive(Parser)]
#[command(
    name = "ttymap",
    about = "Terminal-native scriptable globe — Mapbox Vector Tiles as Braille, scripted with Lua",
    long_about = "ttymap is a terminal-native scriptable globe written in Rust.\n\
        It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters\n\
        with ANSI 256-color in your terminal, on top of a Lua plugin runtime\n\
        for live data overlays, animated camera tours, and custom map UIs.\n\n\
        Inspired by and based on mapscii (https://github.com/rastapasta/mapscii).\n\n\
        Config file:    ~/.config/ttymap/init.lua\n\
        User plugins:   ~/.config/ttymap/lua/plugin/  (activate via `require \"plugin.<name>\"` in init.lua)\n\
        Bundled runtime: ~/.local/share/ttymap/lua/\n\
        Log file:       ~/.local/state/ttymap/ttymap.log\n\
        Tile cache:     ~/.cache/ttymap/",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Subcommand>,

    /// Initial latitude
    #[arg(long)]
    lat: Option<f64>,

    /// Initial longitude
    #[arg(long)]
    lon: Option<f64>,

    /// Initial zoom level
    #[arg(long, short)]
    zoom: Option<f64>,

    /// Style preset (dark, bright)
    #[arg(long)]
    style: Option<String>,

    /// Write debug logs to ~/.local/state/ttymap/ttymap.log. Optional
    /// level argument: `--log` alone is `debug`; `--log info` /
    /// `--log trace` etc. select an explicit level. Without the flag
    /// no logger is installed (log macros become no-ops).
    #[arg(long, value_name = "LEVEL", num_args = 0..=1, default_missing_value = "debug", global = true)]
    log: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Engine-worker is the headless subprocess role. It owns tile
    // cache + render thread + MapState; it doesn't load Lua and
    // doesn't need XDG paths (cache_dir comes via the parent's Init
    // message). Dispatch early so a missing / unresolvable runtime
    // never blocks the worker from booting.
    if matches!(cli.command, Some(Subcommand::EngineWorker)) {
        ttymap_engine::run_as_subprocess();
    }

    // Resolve XDG dirs once for the program (#362). All subsequent
    // consumers (logging, runtime path, lua http / storage, tile
    // cache) read from this single source rather than each calling
    // `ProjectDirs::from("", "", "ttymap")` independently.
    let dirs = AppDirs::resolve();

    // Logging is opt-in via `--log [LEVEL]`. Without the flag no
    // logger is registered and the `log::*!` macros are no-ops.
    // When set, logs land in ~/.local/state/ttymap/ttymap.log
    // (truncated on each launch, so debug sessions don't accumulate).
    // Init failure (state-dir missing, file open denied, logger
    // already installed by something earlier in the process) is
    // surfaced on stderr — the alternative was silent failure where
    // `--log` had no observable effect and no error to grep for.
    if let Some(level) = cli.log.as_deref()
        && let Err(e) = ttymap_app::logging::init(level, dirs.as_ref())
    {
        eprintln!("ttymap: --log requested but logging init failed: {e}");
    }

    // Resolve runtime path before any Lua state spins up. Both the
    // interactive app and the `snap` subcommand reach for it; doing
    // this once at the top means we fail fast with a single error
    // message rather than hitting the same wall in two places.
    let runtime_path = match ttymap_lua::resolve_runtime_path(dirs.as_ref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ttymap: {}", e);
            std::process::exit(1);
        }
    };
    ttymap_lua::set_runtime_path(runtime_path);

    // Subcommands run a single task and exit without booting the full
    // interactive app.
    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.run(dirs) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = run_event_loop(cli, dirs) {
        eprintln!("Error: {e}");
    }
}

/// Composition root: builds the event channel, every subsystem
/// (map / Lua), spawns the off-thread input / frame-timer peers,
/// then hands control to `App::run`. Every thread handle joins
/// in its `Drop` impl, so teardown is just RAII at end of scope.
fn run_event_loop(cli: Cli, dirs: Option<AppDirs>) -> Result<(), Box<dyn std::error::Error>> {
    // Pre-stamp resolved XDG dirs on the seed Config so every
    // subsystem (lua storage / http, engine cache, ...) that reads
    // `config.dirs` during `build_subsystem` sees the same values.
    // `read_back` clones `defaults` and overrides only `opt.*`
    // fields, so dirs flows through transparently.
    let defaults = Config {
        dirs: dirs.clone(),
        ..Default::default()
    };

    // Lua bootstrap runs first — `build_subsystem` creates the VM,
    // installs the API, and runs the init.lua chain (which `require`s
    // every bundled plugin). The tile cache spins up next; its
    // attribution string is fed back into the Lua-side shared cell
    // via `set_attribution` so `ttymap.tile:attribution()` returns
    // the live value.
    let (lua_subsystem, mut config, _keymap_overrides, keymap) =
        ttymap_lua::build_subsystem(defaults);

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

    log::info!(
        "starting ttymap: lat={}, lon={}",
        config.engine.map.lat,
        config.engine.map.lon
    );

    let (event_tx, event_rx) = std::sync::mpsc::channel();

    // Active theme — owned by App, consumed by the map only at
    // construction (initial styler) and on theme switch.
    let theme_id = ttymap_tui::theme::ThemeId::from_name(&config.engine.render.style);

    // Engine doesn't depend on crossterm; the binary owns the
    // terminal-size probe and hands cols/rows to the engine
    // subprocess via the Init handshake.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

    // Spawn the engine as a `ttymap engine-worker` child. The same
    // binary becomes a headless engine in that role (argv-dispatched
    // at the top of `main`), talks to us over stdin/stdout, and
    // streams MapFrames back onto the AppEvent channel via
    // EngineHandle's reader thread. `EngineHandle::Drop` does the
    // cooperative shutdown + child reap at end of scope.
    //
    // `cache_dir` flows in via the Init handshake (#362) — engine
    // itself never resolves XDG paths. `cache.tiles == false` and
    // a missing AppDirs both surface as None here.
    let cache_dir = config
        .dirs
        .as_ref()
        .filter(|_| config.engine.cache.tiles)
        .map(|d| d.cache.clone());
    let map = ttymap_app::engine_handle::EngineHandle::spawn(
        &config.engine,
        cache_dir,
        cols,
        rows,
        theme_id,
        event_tx.clone(),
    )?;

    let lua = lua_subsystem;
    lua.handle.set_attribution(map.attribution.clone());

    // Palette is a built-in (not a plugin): wrap the live Lua
    // registry as a `PaletteIndex` and let `palette::install`
    // build a `CommandSeed` around it, then append the `:`
    // activation to a fresh built-ins Vec. Must run after every
    // plugin's register call so the seed sees them.
    let mut builtin_activations: Vec<ttymap_tui::compositor::Activation> = Vec::new();
    let palette_index: std::rc::Rc<dyn ttymap_tui::compositor::PaletteIndex> =
        std::rc::Rc::new(ttymap_lua::LuaActivationIndex::new(lua.registry.clone()));
    ttymap_tui::palette::install(&keymap, &mut builtin_activations, palette_index);

    let mut app = App::new(config, keymap, theme_id, map, builtin_activations, lua);

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
    // Capture the result so terminal teardown runs on both success and
    // error paths. Without this, an Err from `app.run` would short-
    // circuit via `?` and skip the kitty / mouse-capture / ratatui
    // restore below — leaving the terminal in raw mode + alternate
    // screen while main's error-print fires.
    let run_result = app.run(&mut terminal, &event_rx, &event_tx);
    log::info!("event loop ended");

    if kitty_pushed {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::PopKeyboardEnhancementFlags
        );
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    log::info!("terminal restored, exiting");
    run_result.map_err(Into::into)
}
