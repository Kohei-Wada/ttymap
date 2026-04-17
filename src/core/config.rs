//! Application configuration — struct definitions and TOML file loading.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::Deserialize;

use super::input::Action;
use super::keymap::{KeyMap, parse_key_binding};
use crate::styler::StylePreset;

pub struct Config {
    pub source: String,
    pub style_preset: StylePreset,
    pub initial_lat: f64,
    pub initial_lon: f64,
    pub initial_zoom: Option<f64>,
    pub max_zoom: f64,
    pub zoom_step: f64,
    pub cache_tiles: bool,
    pub language: String,
    pub wiki_limit: u32,
    pub keymap: KeyMap,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: "http://mapscii.me/".to_string(),
            style_preset: StylePreset::Dark,
            initial_lat: 52.51298, // Berlin
            initial_lon: 13.42012,
            initial_zoom: None,
            max_zoom: 18.0,
            zoom_step: 0.2,
            cache_tiles: true,
            language: "en".to_string(),
            wiki_limit: 50,
            keymap: KeyMap::default(),
        }
    }
}

/// Load config from `~/.config/ttymap/config.toml`, merging with defaults.
pub fn load_config() -> Config {
    let mut config = Config::default();

    let path = config_path();
    if let Some(path) = &path
        && let Ok(contents) = fs::read_to_string(path)
    {
        if let Ok(file_cfg) = toml::from_str::<FileConfig>(&contents) {
            file_cfg.apply_to(&mut config);
            log::info!("loaded config from {}", path.display());
        } else {
            log::warn!("failed to parse config file: {}", path.display());
        }
    }

    config
}

fn config_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().join("config.toml"))
}

// ── TOML File Config ──────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct FileConfig {
    source: Option<String>,
    language: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    zoom: Option<f64>,
    zoom_step: Option<f64>,
    max_zoom: Option<f64>,
    style: Option<String>,
    cache_tiles: Option<bool>,
    wiki_limit: Option<u32>,
    keymap: Option<FileKeyMap>,
}

impl FileConfig {
    fn apply_to(&self, config: &mut Config) {
        if let Some(v) = &self.source {
            config.source = v.clone();
        }
        if let Some(v) = &self.language {
            config.language = v.clone();
        }
        if let Some(v) = self.lat {
            config.initial_lat = v;
        }
        if let Some(v) = self.lon {
            config.initial_lon = v;
        }
        if let Some(v) = self.zoom {
            config.initial_zoom = Some(v);
        }
        if let Some(v) = self.zoom_step {
            config.zoom_step = v;
        }
        if let Some(v) = self.max_zoom {
            config.max_zoom = v;
        }
        if let Some(v) = &self.style {
            config.style_preset = match v.as_str() {
                "bright" => StylePreset::Bright,
                _ => StylePreset::Dark,
            };
        }
        if let Some(v) = self.cache_tiles {
            config.cache_tiles = v;
        }
        if let Some(v) = self.wiki_limit {
            config.wiki_limit = v;
        }
        if let Some(km) = &self.keymap {
            km.apply_to(&mut config.keymap);
        }
    }
}

#[derive(Deserialize, Default)]
struct FileKeyMap {
    pan_left: Option<Vec<String>>,
    pan_right: Option<Vec<String>>,
    pan_up: Option<Vec<String>>,
    pan_down: Option<Vec<String>>,
    pan_left_fast: Option<Vec<String>>,
    pan_right_fast: Option<Vec<String>>,
    pan_up_half: Option<Vec<String>>,
    pan_down_half: Option<Vec<String>>,
    zoom_in: Option<Vec<String>>,
    zoom_out: Option<Vec<String>>,
    zoom_to_world: Option<Vec<String>>,
    reset_position: Option<Vec<String>>,
    quit: Option<Vec<String>>,
}

impl FileKeyMap {
    fn apply_to(&self, keymap: &mut KeyMap) {
        let mut map = HashMap::new();
        for (binding, action) in &keymap.bindings {
            map.insert(binding.clone(), action.clone());
        }

        macro_rules! rebind {
            ($field:ident, $action:expr) => {
                if let Some(keys) = &self.$field {
                    map.retain(|_, a| a != &$action);
                    for key_str in keys {
                        if let Some(binding) = parse_key_binding(key_str) {
                            map.insert(binding, $action.clone());
                        } else {
                            log::warn!("invalid key binding: {:?}", key_str);
                        }
                    }
                }
            };
        }

        rebind!(pan_left, Action::PanLeft);
        rebind!(pan_right, Action::PanRight);
        rebind!(pan_up, Action::PanUp);
        rebind!(pan_down, Action::PanDown);
        rebind!(pan_left_fast, Action::PanLeftFast);
        rebind!(pan_right_fast, Action::PanRightFast);
        rebind!(pan_up_half, Action::PanUpHalf);
        rebind!(pan_down_half, Action::PanDownHalf);
        rebind!(zoom_in, Action::ZoomIn);
        rebind!(zoom_out, Action::ZoomOut);
        rebind!(zoom_to_world, Action::ZoomToWorld);
        rebind!(reset_position, Action::ResetPosition);
        rebind!(quit, Action::Quit);

        keymap.bindings = map.into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.max_zoom, 18.0);
        assert!(cfg.source.starts_with("http"));
    }

    #[test]
    fn test_file_config_parsing() {
        let toml_str = r#"
source = "http://example.com/"
language = "ja"
zoom_step = 0.5

[keymap]
zoom_in = ["i"]
quit = ["Q", "C-q"]
"#;
        let file_cfg: FileConfig = toml::from_str(toml_str).unwrap();
        let mut config = Config::default();
        file_cfg.apply_to(&mut config);

        assert_eq!(config.source, "http://example.com/");
        assert_eq!(config.language, "ja");
        assert_eq!(config.zoom_step, 0.5);

        use crossterm::event::KeyModifiers;
        assert_eq!(
            config.keymap.lookup(KeyCode::Char('i'), KeyModifiers::NONE),
            Some(&Action::ZoomIn)
        );
        assert_eq!(
            config.keymap.lookup(KeyCode::Char('a'), KeyModifiers::NONE),
            None
        );
    }
}
