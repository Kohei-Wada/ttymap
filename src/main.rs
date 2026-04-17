use std::fs;

use clap::{Parser, Subcommand};
use termap::app::App;
use termap::core::config;

#[derive(Parser)]
#[command(
    name = "termap",
    about = "Terminal map viewer — renders Mapbox Vector Tiles as Braille characters",
    long_about = "termap is a terminal-based map viewer written in Rust.\n\
        It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters\n\
        with ANSI 256-color in your terminal.\n\n\
        Inspired by and based on mapscii (https://github.com/rastapasta/mapscii).\n\n\
        Config file: ~/.config/termap/config.toml\n\
        Log file:    ~/.local/state/termap/termap.log\n\
        Tile cache:  ~/.cache/termap/",
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
    /// Clear the disk tile cache (~/.cache/termap/)
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

    match termap::logging::init() {
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
        config.style_preset = match v.as_str() {
            "bright" => termap::styler::StylePreset::Bright,
            _ => termap::styler::StylePreset::Dark,
        };
    }

    log::info!(
        "starting termap: lat={}, lon={}",
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
        directories::ProjectDirs::from("", "", "termap").map(|dirs| dirs.cache_dir().to_path_buf());

    match cache_dir {
        Some(dir) if dir.exists() => match fs::remove_dir_all(&dir) {
            Ok(()) => println!("Cleared tile cache: {}", dir.display()),
            Err(e) => eprintln!("Failed to clear cache: {e}"),
        },
        Some(dir) => println!("No cache to clear: {}", dir.display()),
        None => eprintln!("Could not determine cache directory"),
    }
}
