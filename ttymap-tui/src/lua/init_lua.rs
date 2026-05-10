//! `~/.config/ttymap/init.lua` loader — Neovim-style declarative
//! config in Lua, and the **only** entry point for plugin activation.
//!
//! ttymap's runtime is a single Lua VM. The `ttymap` global exposes
//! three layers of surface:
//!
//! ```lua
//! -- ~/.config/ttymap/init.lua
//!
//! ttymap.opt.render.style              = "bright"
//! ttymap.opt.cache.memory_tiles        = 1024
//! ttymap.opt.geoip.on_startup          = true
//! ttymap.opt.runtime.poll_timeout_ms   = 33   -- ~30 Hz
//!
//! ttymap.keymap.set("zoom_in", { "i", "+" })
//! ttymap.keymap.del("pan_left")
//!
//! require "travel"   -- activate a bundled plugin
//! require "myplug"   -- a user lib at ~/.config/ttymap/lua/myplug.lua
//! ```
//!
//! - `ttymap.opt.*` — pre-populated table tree seeded from Rust
//!   defaults. The user mutates leaves (`opt.cache.memory_tiles = N`)
//!   and we read the table back after the chunk runs.
//! - `ttymap.keymap.set/del` — real Lua functions that mutate a
//!   shared [`KeybindingOverrides`] map in Rust.
//! - `require "<name>"` — top-level requires resolve via the
//!   plugin-aware searcher (see [`crate::lua::vm::install_plugin_searcher`]).
//!   On hit, the plugin runs in the shared VM and its `register_*`
//!   calls attribute to `<name>` in the [`PluginRegistry`].
//!
//! Recovery posture matches the rest of the bridge: a missing,
//! unreadable, or throwing `init.lua` logs a warning and the loader
//! returns the unmodified defaults — the app keeps booting.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Lua, Table};

use crate::config::Config;
use crate::input::keymap::KeybindingOverrides;

/// Snapshot config from the init.lua chain WITHOUT installing the
/// plugin runtime API or running plugin requires. Used by the
/// `snap` subcommand which is headless and doesn't need plugins.
///
/// On any error (missing file, IO, Lua syntax), logs a warning and
/// returns the seeded defaults. The app keeps booting.
pub fn read_init_lua_config_only(defaults: Config) -> Config {
    let lua = crate::lua::new_lua();
    let _keymap_state = match install_ttymap_global(&lua, &defaults) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("init.lua: install_ttymap_global failed: {}", e);
            return defaults;
        }
    };
    run_init_lua_chain(&lua);
    read_back(&lua, &defaults).unwrap_or_else(|e| {
        log::warn!("init.lua: read_back failed: {}", e);
        defaults
    })
}

/// Load and execute the bundled + user init.lua sources in `lua`,
/// in that order. Sources that don't exist are skipped silently
/// (debug-logged); IO / Lua errors are warn-logged and the chain
/// keeps going, so a broken bundled init still lets user init run
/// and vice versa.
pub(crate) fn run_init_lua_chain(lua: &Lua) {
    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    if let Some(p) = bundled_init_path()
        && let Some(src) = read_init_file(&p)
    {
        sources.push((src, p));
    }
    if let Some(p) = user_init_path()
        && let Some(src) = read_init_file(&p)
    {
        sources.push((src, p));
    }
    for (source, path) in &sources {
        if let Err(e) = lua
            .load(source)
            .set_name(path.to_string_lossy().as_ref())
            .exec()
        {
            log::warn!("init.lua: {} failed: {}", path.display(), e);
        }
    }
}

/// Read a single init.lua file, logging IO/missing-file outcomes.
/// Returns `None` (treated as "skip this layer") for any failure;
/// the chain keeps walking.
fn read_init_file(path: &Path) -> Option<String> {
    if !path.exists() {
        log::info!("init.lua: not found at {}, skipping", path.display());
        return None;
    }
    match std::fs::read_to_string(path) {
        Ok(s) => {
            log::info!("init.lua: loaded {}", path.display());
            Some(s)
        }
        Err(e) => {
            log::warn!("init.lua: read {} failed: {}", path.display(), e);
            None
        }
    }
}

/// Walk the runtime path looking for the bundled init.lua. Skips
/// the user tier (xdg_config) — that's loaded separately by
/// [`user_init_path`] so user init runs LAST in the chain. Returns
/// the first hit so dev manifest beats stale install (matches the
/// runtime_path priority order).
fn bundled_init_path() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let user = ProjectDirs::from("", "", "ttymap").map(|d| d.config_dir().to_path_buf());
    for layer in crate::lua::runtime_path() {
        if user.as_ref() == Some(layer) {
            continue;
        }
        let candidate = layer.join("init.lua");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Resolve `~/.config/ttymap/init.lua` (or the platform-specific
/// equivalent). `None` only when the host doesn't expose a config
/// dir at all.
fn user_init_path() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().join("init.lua"))
}

/// Build the `ttymap` global with `opt` (pre-populated table tree)
/// and `keymap` (functions backed by the returned state cell).
/// Caller harvests `keymap_state.borrow().clone()` after the init.lua
/// chain runs to fold mutations into a live `KeyMap`.
pub(crate) fn install_ttymap_global(
    lua: &Lua,
    defaults: &Config,
) -> mlua::Result<Rc<RefCell<KeybindingOverrides>>> {
    let keymap_state: Rc<RefCell<KeybindingOverrides>> =
        Rc::new(RefCell::new(KeybindingOverrides::new()));
    let ttymap = lua.create_table()?;
    ttymap.set("opt", build_opt_table(lua, defaults)?)?;
    ttymap.set("keymap", build_keymap_table(lua, keymap_state.clone())?)?;
    lua.globals().set("ttymap", ttymap)?;
    Ok(keymap_state)
}

/// Pre-populate every `Config` field as a Lua table leaf so users
/// can write `ttymap.opt.cache.memory_tiles = 1024` without first
/// having to ensure `cache` exists. Optional fields (e.g.
/// `map.zoom`) are left absent rather than seeded with a sentinel.
fn build_opt_table(lua: &Lua, d: &Config) -> mlua::Result<Table> {
    let opt = lua.create_table()?;

    let map = lua.create_table()?;
    map.set("lat", d.engine.map.lat)?;
    map.set("lon", d.engine.map.lon)?;
    if let Some(z) = d.engine.map.zoom {
        map.set("zoom", z)?;
    }
    map.set("max_zoom", d.engine.map.max_zoom)?;
    map.set("zoom_step", d.engine.map.zoom_step)?;
    opt.set("map", map)?;

    let render = lua.create_table()?;
    render.set("style", d.engine.render.style.clone())?;
    render.set("language", d.engine.render.language.clone())?;
    opt.set("render", render)?;

    let cache = lua.create_table()?;
    cache.set("tiles", d.engine.cache.tiles)?;
    cache.set("memory_tiles", d.engine.cache.memory_tiles)?;
    opt.set("cache", cache)?;

    let geoip = lua.create_table()?;
    geoip.set("on_startup", d.geoip.on_startup)?;
    geoip.set("endpoint", d.geoip.endpoint.clone())?;
    geoip.set("timeout_ms", d.geoip.timeout_ms)?;
    opt.set("geoip", geoip)?;

    let runtime = lua.create_table()?;
    runtime.set("poll_timeout_ms", d.runtime.poll_timeout_ms)?;
    runtime.set("overlay_redraw_ms", d.runtime.overlay_redraw_ms)?;
    runtime.set("sidebar_width", d.runtime.sidebar_width)?;
    opt.set("runtime", runtime)?;

    Ok(opt)
}

/// Build `ttymap.keymap` with `set` and `del` functions backed by
/// `keymap_state`. The functions accept the same shape as the old
/// `[keymap]` TOML section: `set("zoom_in", "i")` or
/// `set("zoom_in", { "i", "+" })`.
fn build_keymap_table(lua: &Lua, state: Rc<RefCell<KeybindingOverrides>>) -> mlua::Result<Table> {
    let keymap = lua.create_table()?;

    let store = state.clone();
    let set = lua.create_function(move |_, (action, keys): (String, mlua::Value)| {
        let keys_vec = keys_to_vec(keys)?;
        store.borrow_mut().insert(action, keys_vec);
        Ok(())
    })?;
    keymap.set("set", set)?;

    let store = state.clone();
    let del = lua.create_function(move |_, action: String| {
        store.borrow_mut().remove(&action);
        Ok(())
    })?;
    keymap.set("del", del)?;

    Ok(keymap)
}

/// Coerce the second argument of `ttymap.keymap.set(action, keys)`
/// into a `Vec<String>`. Accepts a bare string (single binding) or
/// an array of strings (multiple bindings). Anything else is a
/// runtime error reported up to Lua.
fn keys_to_vec(keys: mlua::Value) -> mlua::Result<Vec<String>> {
    match keys {
        mlua::Value::String(s) => Ok(vec![s.to_str()?.to_string()]),
        mlua::Value::Table(t) => Ok(t
            .sequence_values::<String>()
            .filter_map(Result::ok)
            .collect()),
        other => Err(mlua::Error::external(format!(
            "ttymap.keymap.set expected string or array of strings, got {:?}",
            other
        ))),
    }
}

/// Read every `ttymap.opt.*` leaf and merge it into a fresh `Config`
/// seeded with `defaults`. Type errors on any field silently fall
/// back to the seeded value (the field stays at its default). The
/// loader's recovery posture: ttymap doesn't crash because of a
/// `ttymap.opt.cache.memory_tiles = "lots"` typo.
pub(crate) fn read_back(lua: &Lua, defaults: &Config) -> mlua::Result<Config> {
    let ttymap: Table = lua.globals().get("ttymap")?;
    let opt: Table = ttymap.get("opt")?;

    let mut cfg = defaults.clone();

    if let Ok(t) = opt.get::<Table>("map") {
        if let Ok(v) = t.get::<f64>("lat") {
            cfg.engine.map.lat = v;
        }
        if let Ok(v) = t.get::<f64>("lon") {
            cfg.engine.map.lon = v;
        }
        if let Ok(v) = t.get::<Option<f64>>("zoom") {
            cfg.engine.map.zoom = v;
        }
        if let Ok(v) = t.get::<f64>("max_zoom") {
            cfg.engine.map.max_zoom = v;
        }
        if let Ok(v) = t.get::<f64>("zoom_step") {
            cfg.engine.map.zoom_step = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("render") {
        if let Ok(v) = t.get::<String>("style") {
            cfg.engine.render.style = v;
        }
        if let Ok(v) = t.get::<String>("language") {
            cfg.engine.render.language = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("cache") {
        if let Ok(v) = t.get::<bool>("tiles") {
            cfg.engine.cache.tiles = v;
        }
        if let Ok(v) = t.get::<usize>("memory_tiles") {
            cfg.engine.cache.memory_tiles = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("geoip") {
        if let Ok(v) = t.get::<bool>("on_startup") {
            cfg.geoip.on_startup = v;
        }
        if let Ok(v) = t.get::<String>("endpoint") {
            cfg.geoip.endpoint = v;
        }
        if let Ok(v) = t.get::<u64>("timeout_ms") {
            cfg.geoip.timeout_ms = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("runtime") {
        if let Ok(v) = t.get::<u64>("poll_timeout_ms") {
            cfg.runtime.poll_timeout_ms = v;
        }
        if let Ok(v) = t.get::<u64>("overlay_redraw_ms") {
            cfg.runtime.overlay_redraw_ms = v;
        }
        if let Ok(v) = t.get::<u16>("sidebar_width") {
            cfg.runtime.sidebar_width = v;
        }
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run an init.lua-style source against a fresh VM with `ttymap`
    /// pre-pass installed (no plugin API), then read config back.
    /// Mirrors what the tests exercised before the unified bootstrap.
    fn run(source: &str) -> (Config, KeybindingOverrides) {
        crate::lua::runtimepath::ensure_runtime_path_for_tests();
        let lua = crate::lua::new_lua();
        let defaults = Config::default();
        let keymap_state = install_ttymap_global(&lua, &defaults).expect("install ttymap");
        if let Err(e) = lua.load(source).set_name("test").exec() {
            log::warn!("init.lua test source failed: {}", e);
        }
        let cfg = read_back(&lua, &defaults).expect("read_back");
        let km = keymap_state.borrow().clone();
        (cfg, km)
    }

    #[test]
    fn empty_init_returns_defaults() {
        let (cfg, km) = run("");
        let d = Config::default();
        assert_eq!(cfg.engine.render.style, d.engine.render.style);
        assert_eq!(cfg.engine.cache.memory_tiles, d.engine.cache.memory_tiles);
        assert!(km.is_empty());
    }

    #[test]
    fn opt_render_style_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.render.style = "bright""#);
        assert_eq!(cfg.engine.render.style, "bright");
    }

    #[test]
    fn opt_cache_memory_tiles_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.cache.memory_tiles = 1024"#);
        assert_eq!(cfg.engine.cache.memory_tiles, 1024);
    }

    #[test]
    fn opt_map_zoom_optional_can_be_set() {
        let (cfg, _) = run(r#"ttymap.opt.map.zoom = 10"#);
        assert_eq!(cfg.engine.map.zoom, Some(10.0));
    }

    #[test]
    fn opt_geoip_full_section_takes_effect() {
        let src = r#"
            ttymap.opt.geoip.on_startup = true
            ttymap.opt.geoip.endpoint   = "https://example.com/ip"
            ttymap.opt.geoip.timeout_ms = 500
        "#;
        let (cfg, _) = run(src);
        assert!(cfg.geoip.on_startup);
        assert_eq!(cfg.geoip.endpoint, "https://example.com/ip");
        assert_eq!(cfg.geoip.timeout_ms, 500);
    }

    #[test]
    fn keymap_set_with_string_records_single_binding() {
        let (_, km) = run(r#"ttymap.keymap.set("zoom_in", "i")"#);
        assert_eq!(
            km.get("zoom_in").map(|v| v.as_slice()),
            Some(&["i".to_string()][..])
        );
    }

    #[test]
    fn keymap_set_with_table_records_array() {
        let (_, km) = run(r#"ttymap.keymap.set("quit", { "Q", "C-q" })"#);
        assert_eq!(
            km.get("quit").map(|v| v.as_slice()),
            Some(&["Q".to_string(), "C-q".to_string()][..])
        );
    }

    #[test]
    fn keymap_del_removes_a_set() {
        let src = r#"
            ttymap.keymap.set("zoom_in", "i")
            ttymap.keymap.del("zoom_in")
        "#;
        let (_, km) = run(src);
        assert!(km.is_empty());
    }

    #[test]
    fn type_error_on_opt_field_falls_back_to_default() {
        // Setting a string where an int is expected — must not crash;
        // the field stays at its default.
        let src = r#"ttymap.opt.cache.memory_tiles = "lots""#;
        let (cfg, _) = run(src);
        assert_eq!(
            cfg.engine.cache.memory_tiles,
            Config::default().engine.cache.memory_tiles
        );
    }

    #[test]
    fn programmatic_config_works_in_lua() {
        // Conditional / computed values — the killer feature over TOML.
        let src = r#"
            local heavy = true
            ttymap.opt.cache.memory_tiles = heavy and 2048 or 256
        "#;
        let (cfg, _) = run(src);
        assert_eq!(cfg.engine.cache.memory_tiles, 2048);
    }

    #[test]
    fn opt_runtime_poll_timeout_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.runtime.poll_timeout_ms = 33"#);
        assert_eq!(cfg.runtime.poll_timeout_ms, 33);
    }

    #[test]
    fn opt_runtime_overlay_redraw_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.runtime.overlay_redraw_ms = 200"#);
        assert_eq!(cfg.runtime.overlay_redraw_ms, 200);
    }
}
