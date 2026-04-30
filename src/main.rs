use clap::Parser;
use ttymap::app::App;
use ttymap::commands::Command as Subcommand;
use ttymap::config;

#[derive(Parser)]
#[command(
    name = "ttymap",
    about = "Terminal map viewer — renders Mapbox Vector Tiles as Braille characters",
    long_about = "ttymap is a terminal-based map viewer written in Rust.\n\
        It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters\n\
        with ANSI 256-color in your terminal.\n\n\
        Inspired by and based on mapscii (https://github.com/rastapasta/mapscii).\n\n\
        Config file:    ~/.config/ttymap/config.toml\n\
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

    // Subcommands run a single task and exit without booting the full
    // interactive app.
    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.run() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Load config file first, then override with CLI args
    let mut config = config::load_config();

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

    let mut app = App::new(config);
    if let Err(e) = app.run() {
        eprintln!("Error: {e}");
    }
}
