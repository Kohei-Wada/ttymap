//! Application configuration — `Config` is both the runtime
//! representation and the TOML file schema. Missing fields fall back
//! to `Config::default()` via `#[serde(default)]`, so a partially
//! written `config.toml` picks up sane values for everything else.
//!
//! The `[keymap]` section deserialises into `KeybindingOverrides`
//! (defined in `keymap.rs` alongside the `KeyMap` it configures);
//! this module stays focused on "parse TOML into ergonomic data".

use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::Deserialize;

pub use crate::keymap::KeybindingOverrides;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    /// Visual theme name ("dark" / "bright"). Unknown values fall
    /// back to a default at styler-initialisation time.
    pub style: String,
    #[serde(rename = "lat")]
    pub initial_lat: f64,
    #[serde(rename = "lon")]
    pub initial_lon: f64,
    #[serde(rename = "zoom")]
    pub initial_zoom: Option<f64>,
    pub max_zoom: f64,
    pub zoom_step: f64,
    pub cache_tiles: bool,
    pub language: String,
    pub wiki_limit: u32,
    /// Jump to IP-based location on startup (can also be enabled by `--here`).
    pub here_on_startup: bool,
    /// IP geolocation endpoint. Must return JSON with `latitude`/`longitude`
    /// numeric fields (ipapi.co shape).
    pub geoip_endpoint: String,
    /// Timeout for the IP geolocation request, in milliseconds.
    pub geoip_timeout_ms: u64,
    pub keymap: KeybindingOverrides,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            style: "dark".to_string(),
            initial_lat: 52.51298, // Berlin
            initial_lon: 13.42012,
            initial_zoom: None,
            max_zoom: 18.0,
            zoom_step: 0.2,
            cache_tiles: true,
            language: "en".to_string(),
            wiki_limit: 50,
            here_on_startup: false,
            geoip_endpoint: "https://ipapi.co/json/".to_string(),
            geoip_timeout_ms: 2000,
            keymap: KeybindingOverrides::default(),
        }
    }
}

/// Load config from `~/.config/ttymap/config.toml`. Returns defaults
/// if the file is missing or malformed.
pub fn load_config() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return Config::default();
    };
    match toml::from_str::<Config>(&contents) {
        Ok(cfg) => {
            log::info!("loaded config from {}", path.display());
            cfg
        }
        Err(e) => {
            log::warn!("failed to parse {}: {e}", path.display());
            Config::default()
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.max_zoom, 18.0);
        assert_eq!(cfg.style, "dark");
    }

    #[test]
    fn test_partial_toml_fills_defaults_elsewhere() {
        let toml_str = r#"
language = "ja"
zoom_step = 0.5
style = "bright"
here_on_startup = true
geoip_timeout_ms = 500

[keymap]
zoom_in = ["i"]
quit = ["Q", "C-q"]
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();

        // Overridden.
        assert_eq!(cfg.language, "ja");
        assert_eq!(cfg.zoom_step, 0.5);
        assert_eq!(cfg.style, "bright");
        assert!(cfg.here_on_startup);
        assert_eq!(cfg.geoip_timeout_ms, 500);

        // Unspecified fields kept their defaults.
        assert_eq!(cfg.max_zoom, 18.0);
        assert_eq!(cfg.initial_lat, 52.51298);
        assert_eq!(cfg.geoip_endpoint, "https://ipapi.co/json/");

        // Keymap overrides are stored raw; resolution to KeyMap is in app.rs.
        assert_eq!(cfg.keymap.zoom_in.as_deref(), Some(&["i".to_string()][..]));
        assert_eq!(
            cfg.keymap.quit.as_deref(),
            Some(&["Q".to_string(), "C-q".to_string()][..])
        );
    }

    #[test]
    fn test_empty_toml_is_all_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        let def = Config::default();
        assert_eq!(cfg.initial_lat, def.initial_lat);
        assert_eq!(cfg.max_zoom, def.max_zoom);
    }
}
