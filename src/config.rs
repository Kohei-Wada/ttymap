//! Application configuration — `Config` is both the runtime
//! representation and the TOML file schema. Each sub-struct is a
//! `[section]` in the TOML file; every field has a serde default so
//! partial files stay valid. Section defaults apply even when the
//! section header is omitted.
//!
//! The `[keymap]` section deserialises into `KeybindingOverrides`
//! (defined in `keymap.rs` alongside the `KeyMap` it configures);
//! this module stays focused on "parse TOML into ergonomic data".
//!
//! ## Plugin configuration
//!
//! All in-tree plugins are now Lua (see `src/lua/scripts/`). Plugin
//! settings live as constants inside each `.lua` script — there is
//! no per-plugin TOML knob and no `[<name>] enabled = false` toggle.
//! The `extras` catch-all is kept only so old `config.toml` files
//! that still mention plugin sections parse without error; the
//! captured TOML is otherwise unused.

use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::Deserialize;

pub use crate::keymap::KeybindingOverrides;

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub map: MapConfig,
    pub render: RenderConfig,
    pub cache: CacheConfig,
    pub geoip: GeoipConfig,
    pub keymap: KeybindingOverrides,
    /// Catch-all for unrecognised TOML sections. Kept for backward
    /// compatibility with existing `config.toml` files that still
    /// mention `[aircraft]`, `[wiki]`, etc. — there are no readers
    /// today, so the captured table is silently ignored.
    #[serde(flatten)]
    #[allow(dead_code)]
    pub extras: toml::Table,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct MapConfig {
    pub lat: f64,
    pub lon: f64,
    pub zoom: Option<f64>,
    pub max_zoom: f64,
    pub zoom_step: f64,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            lat: 52.51298, // Berlin
            lon: 13.42012,
            zoom: None,
            max_zoom: 18.0,
            zoom_step: 0.2,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// Visual theme name ("dark" / "bright"). Unknown values fall
    /// back to a default at styler-initialisation time.
    pub style: String,
    pub language: String,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            style: "dark".to_string(),
            language: "en".to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Write decoded tiles to `~/.cache/ttymap/` so they survive restarts.
    pub tiles: bool,
    /// Decoded-tile LRU capacity. Each "view" (visible 9 + prefetch
    /// z±1) costs ~22 tiles; sized to keep a handful of recently-
    /// visited views resident across pan and zoom-step churn so a
    /// quick zoom-in / zoom-out doesn't re-fetch every level.
    /// Raise further if working with very large viewports or long
    /// pan trails.
    pub memory_tiles: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            tiles: true,
            // 22 tiles/view × ~23 distinct views ≈ 512. With the old
            // 192 default a fast zoom across ~9 levels exhausted the
            // LRU and evicted earlier levels mid-flight, producing
            // visible black squares on zoom-back.
            memory_tiles: 512,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct GeoipConfig {
    /// Jump to IP-based location on startup (can also be enabled by `--here`).
    pub on_startup: bool,
    /// IP geolocation endpoint. Must return JSON with `latitude`/`longitude`
    /// numeric fields (ipapi.co shape).
    pub endpoint: String,
    /// Timeout for the IP geolocation request, in milliseconds.
    pub timeout_ms: u64,
}

impl Default for GeoipConfig {
    fn default() -> Self {
        Self {
            on_startup: false,
            endpoint: "https://ipapi.co/json/".to_string(),
            timeout_ms: 2000,
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
        assert_eq!(cfg.map.max_zoom, 18.0);
        assert_eq!(cfg.render.style, "dark");
        assert_eq!(cfg.cache.memory_tiles, 512);
    }

    #[test]
    fn test_partial_toml_fills_defaults_elsewhere() {
        let toml_str = r#"
[map]
zoom_step = 0.5

[render]
language = "ja"
style = "bright"

[geoip]
on_startup = true
timeout_ms = 500

[cache]
memory_tiles = 256

[keymap]
zoom_in = ["i"]
quit = ["Q", "C-q"]
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();

        // Overridden.
        assert_eq!(cfg.render.language, "ja");
        assert_eq!(cfg.map.zoom_step, 0.5);
        assert_eq!(cfg.render.style, "bright");
        assert!(cfg.geoip.on_startup);
        assert_eq!(cfg.geoip.timeout_ms, 500);
        assert_eq!(cfg.cache.memory_tiles, 256);

        // Unspecified fields kept their defaults.
        assert_eq!(cfg.map.max_zoom, 18.0);
        assert_eq!(cfg.map.lat, 52.51298);
        assert_eq!(cfg.geoip.endpoint, "https://ipapi.co/json/");
        assert!(cfg.cache.tiles);

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
        assert_eq!(cfg.map.lat, def.map.lat);
        assert_eq!(cfg.map.max_zoom, def.map.max_zoom);
        assert_eq!(cfg.cache.memory_tiles, def.cache.memory_tiles);
    }

    #[test]
    fn test_missing_section_headers_use_section_defaults() {
        // Omitting a section header entirely should still give that
        // section its default — serde(default) on each sub-struct field
        // is what makes this work.
        let cfg: Config = toml::from_str(r#"[keymap]"#).unwrap();
        assert_eq!(cfg.render.style, "dark");
        assert_eq!(cfg.cache.memory_tiles, 512);
        assert!(!cfg.geoip.on_startup);
    }
}
