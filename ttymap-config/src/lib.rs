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

pub mod dirs;

use std::collections::HashMap;

pub use dirs::AppDirs;
pub use ttymap_engine::config::{CacheConfig, MapConfig, RenderConfig};

/// Raw keybinding overrides built up from `ttymap.keymap.set(...)` /
/// `ttymap.keymap.del(...)` calls in `init.lua`. Keys are
/// `MapAction::config_name` strings (e.g. `"pan_left"`); values
/// replace the default bindings for that action (wrapped as
/// `UserCommand::Map` internally). Folded into a live
/// `ttymap_tui::input::KeyMap` via `KeyMap::with_overrides`.
///
/// Lives in `ttymap-config` (not core or tui) because keybindings
/// are user-supplied settings — same conceptual category as
/// [`RuntimeConfig`] fields, just expressed as a name → keys map
/// rather than struct fields. `ttymap-tui::input::KeyMap` accepts
/// the raw `HashMap<String, Vec<String>>` shape directly so it
/// doesn't have to depend on this crate.
pub type KeybindingOverrides = HashMap<String, Vec<String>>;

#[derive(Clone)]
pub struct Config {
    /// Engine-side settings consumed by the map / render pipeline.
    pub engine: ttymap_engine::Config,
    pub runtime: RuntimeConfig,
    /// Resolved XDG directories for the `ttymap` brand. `None` only
    /// in pathological environments where `directories::ProjectDirs`
    /// can't find a home (CI sandboxes with `$HOME` unset, mostly).
    /// The composition root (`ttymap-app/src/main.rs`,
    /// `ttymap-cli/src/snap.rs`) calls `AppDirs::resolve()` once at
    /// startup and pre-stamps this field before any subsystem boots,
    /// so consumers (engine cache, lua http / storage, runtime path
    /// resolver, log file) read the same paths.
    pub dirs: Option<AppDirs>,
}

impl Default for Config {
    fn default() -> Self {
        // Initial viewport is binary policy, not engine concern —
        // `ttymap_engine::MapConfig::default()` ships `lat/lon: None`.
        // We seed Berlin here so both `ttymap-app` and `ttymap-cli`
        // (snap) inherit it via `Config::default()` without having
        // to repeat the constant. Users override via `--lat/--lon`,
        // `--here`, or `ttymap.opt.{lat,lon}` in init.lua.
        let mut engine = ttymap_engine::Config::default();
        engine.map.lat = Some(52.51298);
        engine.map.lon = Some(13.42012);
        Self {
            engine,
            runtime: RuntimeConfig::default(),
            dirs: None,
        }
    }
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

    #[test]
    fn default_seeds_berlin_viewport_at_config_layer() {
        // Engine itself has no viewport opinion; the Berlin seed
        // lives in this crate's `Config::default` so both
        // `ttymap-app` and `ttymap-cli` (snap) inherit it.
        let cfg = Config::default();
        assert_eq!(cfg.engine.map.lat, Some(52.51298));
        assert_eq!(cfg.engine.map.lon, Some(13.42012));
        assert_eq!(ttymap_engine::Config::default().map.lat, None);
        assert_eq!(ttymap_engine::Config::default().map.lon, None);
    }
}
