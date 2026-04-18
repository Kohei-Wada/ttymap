use std::fs;

use clap::{Parser, Subcommand};
use ttymap::app::App;
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
    command: Option<Command>,

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

    /// Tile source URL (default: http://mapscii.me/)
    #[arg(long)]
    source: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Clear the disk tile cache (~/.cache/ttymap/)
    ClearCache,
}

fn main() {
    let cli = Cli::parse();

    // Handle subcommands that don't need the full app
    if let Some(cmd) = &cli.command {
        match cmd {
            Command::ClearCache => {
                clear_cache();
                return;
            }
        }
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
    if let Some(v) = cli.source {
        config.source = v;
    }
    if let Some(v) = cli.style {
        // Unknown values get normalised to "dark" by the styler's
        // fallback at construction time; just hand the raw string in.
        config.style = v;
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

fn clear_cache() {
    let cache_dir =
        directories::ProjectDirs::from("", "", "ttymap").map(|dirs| dirs.cache_dir().to_path_buf());

    match cache_dir {
        Some(dir) if dir.exists() => match fs::remove_dir_all(&dir) {
            Ok(()) => println!("Cleared tile cache: {}", dir.display()),
            Err(e) => eprintln!("Failed to clear cache: {e}"),
        },
        Some(dir) => println!("No cache to clear: {}", dir.display()),
        None => eprintln!("Could not determine cache directory"),
    }
}
