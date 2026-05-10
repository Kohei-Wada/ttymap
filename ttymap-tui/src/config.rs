//! Application configuration — the runtime-shape struct populated
//! from `~/.config/ttymap/init.lua`. Each sub-struct used to be a
//! `[section]` in a TOML file; the schema didn't change in shape
//! when we migrated to Lua, only in the *language* the user writes
//! their overrides in.
//!
//! Split with the engine: [`ttymap_engine::Config`] owns the
//! map/render/cache subset the rendering engine actually consumes;
//! this struct wraps it with binary-only knobs (runtime,
//! keybinding overrides). Engine-side fields are reached via
//! `config.engine.<sub>.<field>`.
//!
//! The actual loader lives in
//! [`crate::lua::build_subsystem`] (and the snap-only
//! `read_init_lua_config_only` helper in
//! [`crate::lua::init_lua`]). This module just owns the struct
//! definitions and their `Default` impls (which act as the seed
//! Lua starts from).

pub use crate::input::keymap::KeybindingOverrides;
pub use ttymap_engine::config::{CacheConfig, MapConfig, RenderConfig};

#[derive(Default, Clone)]
pub struct Config {
    /// Engine-side settings consumed by the map / render pipeline.
    pub engine: ttymap_engine::Config,
    pub runtime: RuntimeConfig,
}

#[derive(Clone)]
pub struct RuntimeConfig {
    /// Main event-loop wake interval in milliseconds. Lower = more
    /// responsive input and smoother animation but higher idle CPU.
    /// 50 ms (20 Hz) balances input-latency imperceptibility against
    /// per-tick `ui::draw` cost.
    pub poll_timeout_ms: u64,
    /// Minimum interval between overlay-driven redraws in
    /// milliseconds. Plugins can push polylines every tick at the
    /// poll rate; the App rate-limits the resulting full-tile re-
    /// renders to this interval. Lower = smoother animation but
    /// higher render-thread CPU. 100 ms (10 Hz) is enough for typical
    /// growing-line animations.
    pub overlay_redraw_ms: u64,
    /// Width (terminal cells) of the left sidebar when toggled
    /// visible. Default 56 matches the bundled `wiki` / `aircraft`
    /// modal panel widths so sidebar-hosted plugin sections look
    /// the same as their floating-panel counterparts. Configurable
    /// from Lua: `ttymap.opt.runtime.sidebar_width = 60`.
    pub sidebar_width: u16,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            poll_timeout_ms: 50,
            overlay_redraw_ms: 100,
            sidebar_width: 56,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_expected_seeds() {
        let cfg = Config::default();
        assert_eq!(cfg.engine.map.max_zoom, 18.0);
        assert_eq!(cfg.engine.render.style, "dark");
        assert_eq!(cfg.engine.cache.memory_tiles, 512);
        assert_eq!(cfg.runtime.poll_timeout_ms, 50);
        assert_eq!(cfg.runtime.overlay_redraw_ms, 100);
        assert_eq!(cfg.runtime.sidebar_width, 56);
    }
}
