//! Engine-side configuration.
//!
//! Subset of the binary's settings that the rendering engine actually
//! consumes: tile cache, initial viewport, render style/language. The
//! binary wraps this with its own runtime (poll timeout / sidebar
//! width / …), geoip, and plugin-disable list.
//!
//! No I/O lives here — `Default` is the seed; the binary populates
//! the live values from `init.lua` via `crate::lua::load_init_lua`.

#[derive(Default, Clone)]
pub struct Config {
    pub cache: CacheConfig,
    pub map: MapConfig,
    pub render: RenderConfig,
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
            memory_tiles: 512,
        }
    }
}
