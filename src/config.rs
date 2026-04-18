//! Application configuration — `Config` is both the runtime
//! representation and the TOML file schema. Missing fields fall back
//! to `Config::default()` via `#[serde(default)]`, so a partially
//! written `config.toml` picks up sane values for everything else.
//!
//! Resolution of raw keybindings into a concrete `KeyMap` lives in the
//! app layer (`app.rs::build_keymap`); this module stays focused on
//! "parse TOML into ergonomic data".

use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::Deserialize;

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

/// Raw keybinding overrides from the `[keymap]` section of
/// `config.toml`. Each field names an `Action`; the listed key strings
/// replace the default bindings for that action. `app.rs::build_keymap`
/// resolves this into a `KeyMap`.
#[derive(Deserialize, Default, Clone)]
pub struct KeybindingOverrides {
    pub pan_left: Option<Vec<String>>,
    pub pan_right: Option<Vec<String>>,
    pub pan_up: Option<Vec<String>>,
    pub pan_down: Option<Vec<String>>,
    pub pan_left_fast: Option<Vec<String>>,
    pub pan_right_fast: Option<Vec<String>>,
    pub pan_up_half: Option<Vec<String>>,
    pub pan_down_half: Option<Vec<String>>,
    pub zoom_in: Option<Vec<String>>,
    pub zoom_out: Option<Vec<String>>,
    pub zoom_to_world: Option<Vec<String>>,
    pub reset_position: Option<Vec<String>>,
    pub quit: Option<Vec<String>>,
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

[keymap]
zoom_in = ["i"]
quit = ["Q", "C-q"]
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();

        // Overridden.
        assert_eq!(cfg.language, "ja");
        assert_eq!(cfg.zoom_step, 0.5);
        assert_eq!(cfg.style, "bright");

        // Unspecified fields kept their defaults.
        assert_eq!(cfg.max_zoom, 18.0);
        assert_eq!(cfg.initial_lat, 52.51298);

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
