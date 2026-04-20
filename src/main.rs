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
        Config file: ~/.config/ttymap/config.toml\n\
        Log file:    ~/.local/state/ttymap/ttymap.log\n\
        Tile cache:  ~/.cache/ttymap/",
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

    // Subcommands run a single task and exit without booting the full
    // interactive app.
    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.run() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    match ttymap::logging::init() {
        Ok(path) => eprintln!("logging to {}", path.display()),
        Err(e) => eprintln!("warning: could not initialize logging: {e}"),
    }

    // Load config file first, then override with CLI args
    let mut config = config::load_config();

    if let Some(v) = cli.lat {
        config.initial_lat = v;
    }
    if let Some(v) = cli.lon {
        config.initial_lon = v;
    }
    if let Some(v) = cli.zoom {
        config.initial_zoom = Some(v);
    }
    if let Some(v) = cli.style {
        // Unknown values get normalised to "dark" by the styler's
        // fallback at construction time; just hand the raw string in.
        config.style = v;
    }

    if cli.here || config.here_on_startup {
        match ttymap::shared::geoip::lookup(&config.geoip_endpoint, config.geoip_timeout_ms) {
            Some((lat, lon)) => {
                log::info!("geoip: resolved to {}, {}", lat, lon);
                config.initial_lat = lat;
                config.initial_lon = lon;
            }
            None => {
                log::warn!(
                    "geoip lookup failed, using default {}, {}",
                    config.initial_lat,
                    config.initial_lon
                );
            }
        }
    }

    log::info!(
        "starting ttymap: lat={}, lon={}",
        config.initial_lat,
        config.initial_lon
    );

    let mut app = App::new(config);
    if let Err(e) = app.run() {
        eprintln!("Error: {e}");
    }
}
