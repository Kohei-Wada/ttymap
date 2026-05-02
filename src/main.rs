use clap::Parser;
use ttymap::app::App;
use ttymap::commands::Command as Subcommand;
use ttymap::config::Config;

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

    // Build the event channel + bus + App as separate concerns at
    // the composition root. The bus is no longer something `App`
    // owns — App is one participant on it (state mutation +
    // notification emit), peer to the Lua plugin subscribers and
    // the off-thread sources (render / input / frame timer).
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let (mut app, event_bus) = App::new(config, keymap_overrides, event_tx.clone());
    if let Err(e) = app.run(event_rx, event_tx, &event_bus) {
        eprintln!("Error: {e}");
    }
}
