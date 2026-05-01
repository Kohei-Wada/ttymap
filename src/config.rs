//! Application configuration — the runtime-shape struct populated
//! from `~/.config/ttymap/init.lua`. Each sub-struct used to be a
//! `[section]` in a TOML file; the schema didn't change in shape
//! when we migrated to Lua, only in the *language* the user writes
//! their overrides in.
//!
//! The actual loader lives in [`crate::lua::init_lua::run_init_lua`].
//! This module just owns the struct definitions and their `Default`
//! impls (which act as the seed Lua starts from).

pub use crate::keymap::KeybindingOverrides;

#[derive(Default, Clone)]
pub struct Config {
    pub map: MapConfig,
    pub render: RenderConfig,
    pub cache: CacheConfig,
    pub geoip: GeoipConfig,
    pub plugins: PluginsConfig,
}

#[derive(Default, Clone)]
pub struct PluginsConfig {
    /// User-supplied opt-out list, matched against each plugin's
    /// stem (file name minus `.lua`). Set via
    /// `ttymap.opt.plugins.disable = { "wiki", "quake" }` in
    /// init.lua. Plugins matching any entry are silently skipped at
    /// registration time.
    pub disable: Vec<String>,
}

#[derive(Clone)]
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

#[derive(Clone)]
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

#[derive(Clone)]
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

#[derive(Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_expected_seeds() {
        let cfg = Config::default();
        assert_eq!(cfg.map.max_zoom, 18.0);
        assert_eq!(cfg.render.style, "dark");
        assert_eq!(cfg.cache.memory_tiles, 512);
    }
}
