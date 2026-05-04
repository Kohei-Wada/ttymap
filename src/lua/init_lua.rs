//! `~/.config/ttymap/init.lua` loader — Neovim-style declarative
//! config in Lua.
//!
//! Replaces the old `config.toml` parse path. The Lua side exposes
//! a `ttymap` global with two namespaces:
//!
//! ```lua
//! -- ~/.config/ttymap/init.lua
//!
//! ttymap.opt.render.style              = "bright"
//! ttymap.opt.cache.memory_tiles        = 1024
//! ttymap.opt.geoip.on_startup          = true
//! ttymap.opt.runtime.poll_timeout_ms   = 33   -- ~30 Hz
//! ttymap.opt.runtime.overlay_redraw_ms = 50   -- smoother overlays
//!
//! ttymap.keymap.set("zoom_in", { "i", "+" })
//! ttymap.keymap.del("pan_left")
//! ```
//!
//! - `ttymap.opt.*` is a pre-populated table tree seeded from Rust
//!   defaults. The user mutates leaves (`opt.cache.memory_tiles = N`)
//!   and we read the table back after the chunk runs.
//! - `ttymap.keymap.set(action, keys)` / `ttymap.keymap.del(action)`
//!   are real Lua functions that mutate a shared
//!   [`KeybindingOverrides`] map in Rust.
//!
//! Recovery posture matches the rest of the bridge: a missing,
//! unreadable, or throwing `init.lua` logs a warning and the loader
//! returns the unmodified defaults — the app keeps booting.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Lua, Table};

use crate::config::Config;
use crate::core::input::keymap::KeybindingOverrides;

/// Run the init.lua chain against `defaults` and return the
/// resulting `(Config, KeybindingOverrides)`. nvim-style sysinit:
///
/// 1. Bundled defaults at `<bundled-tier>/init.lua` (first hit
///    among env / manifest / xdg_data) — ships with ttymap, edits
///    here would land via PR.
/// 2. User overrides at `<xdg_config>/init.lua` (e.g.
///    `~/.config/ttymap/init.lua`).
///
/// Both files run in the same Lua state, in that order, so the
/// user's mutations override the bundled side via simple last-wins
/// on the shared `ttymap.opt.*` table and `ttymap.keymap` map.
///
/// Failure modes (missing file, IO error, Lua syntax/runtime error)
/// all log + leave the prior state intact. A broken bundled init
/// still lets the user's init.lua run; a broken user init still
/// lets the bundled defaults take effect.
pub fn load_init_lua(defaults: Config) -> (Config, KeybindingOverrides) {
    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    if let Some(p) = bundled_init_path()
        && let Some(src) = read_init_file(&p)
    {
        sources.push((src, p));
    }
    if let Some(p) = init_lua_path()
        && let Some(src) = read_init_file(&p)
    {
        sources.push((src, p));
    }

    if sources.is_empty() {
        return (defaults, KeybindingOverrides::new());
    }

    match exec(&sources, &defaults) {
        Ok((cfg, km)) => (cfg, km),
        Err(e) => {
            log::warn!("init.lua: chain failed: {}", e);
            (defaults, KeybindingOverrides::new())
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
/// `init_lua_path` so user init runs LAST in the chain. Returns
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

/// Inner half of [`load_init_lua`] — pure logic split out so unit
/// tests can drive Lua source directly without faking the XDG path.
/// Each source in `sources` runs in turn against the same Lua state;
/// `ttymap.opt.*` mutations and `ttymap.keymap.set/del` calls
/// accumulate, with the last source winning on conflicts.
pub(crate) fn exec(
    sources: &[(String, PathBuf)],
    defaults: &Config,
) -> mlua::Result<(Config, KeybindingOverrides)> {
    let lua = crate::lua::new_lua();
    let keymap_state: Rc<RefCell<KeybindingOverrides>> =
        Rc::new(RefCell::new(KeybindingOverrides::new()));
    install_ttymap_global(&lua, defaults, keymap_state.clone())?;
    for (source, path) in sources {
        if let Err(e) = lua
            .load(source)
            .set_name(path.to_string_lossy().as_ref())
            .exec()
        {
            log::warn!("init.lua: {} failed: {}", path.display(), e);
        }
    }
    let cfg = read_back(&lua, defaults)?;
    // Drop the Lua state so the only surviving Rc clone is ours.
    drop(lua);
    let keymap = Rc::try_unwrap(keymap_state)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| rc.borrow().clone());
    Ok((cfg, keymap))
}

/// Build the `ttymap` global with `opt` (pre-populated table tree)
/// and `keymap` (functions backed by `keymap_state`).
fn install_ttymap_global(
    lua: &Lua,
    defaults: &Config,
    keymap_state: Rc<RefCell<KeybindingOverrides>>,
) -> mlua::Result<()> {
    let ttymap = lua.create_table()?;
    ttymap.set("opt", build_opt_table(lua, defaults)?)?;
    ttymap.set("keymap", build_keymap_table(lua, keymap_state)?)?;
    lua.globals().set("ttymap", ttymap)?;
    Ok(())
}

/// Pre-populate every `Config` field as a Lua table leaf so users
/// can write `ttymap.opt.cache.memory_tiles = 1024` without first
/// having to ensure `cache` exists. Optional fields (e.g.
/// `map.zoom`) are left absent rather than seeded with a sentinel.
fn build_opt_table(lua: &Lua, d: &Config) -> mlua::Result<Table> {
    let opt = lua.create_table()?;

    let map = lua.create_table()?;
    map.set("lat", d.map.lat)?;
    map.set("lon", d.map.lon)?;
    if let Some(z) = d.map.zoom {
        map.set("zoom", z)?;
    }
    map.set("max_zoom", d.map.max_zoom)?;
    map.set("zoom_step", d.map.zoom_step)?;
    opt.set("map", map)?;

    let render = lua.create_table()?;
    render.set("style", d.render.style.clone())?;
    render.set("language", d.render.language.clone())?;
    opt.set("render", render)?;

    let cache = lua.create_table()?;
    cache.set("tiles", d.cache.tiles)?;
    cache.set("memory_tiles", d.cache.memory_tiles)?;
    opt.set("cache", cache)?;

    let geoip = lua.create_table()?;
    geoip.set("on_startup", d.geoip.on_startup)?;
    geoip.set("endpoint", d.geoip.endpoint.clone())?;
    geoip.set("timeout_ms", d.geoip.timeout_ms)?;
    opt.set("geoip", geoip)?;

    let disable = lua.create_table()?;
    for (i, name) in d.plugins.disable.iter().enumerate() {
        disable.set(i + 1, name.as_str())?;
    }
    opt.set("disable", disable)?;

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
fn read_back(lua: &Lua, defaults: &Config) -> mlua::Result<Config> {
    let ttymap: Table = lua.globals().get("ttymap")?;
    let opt: Table = ttymap.get("opt")?;

    let mut cfg = defaults.clone();

    if let Ok(t) = opt.get::<Table>("map") {
        if let Ok(v) = t.get::<f64>("lat") {
            cfg.map.lat = v;
        }
        if let Ok(v) = t.get::<f64>("lon") {
            cfg.map.lon = v;
        }
        if let Ok(v) = t.get::<Option<f64>>("zoom") {
            cfg.map.zoom = v;
        }
        if let Ok(v) = t.get::<f64>("max_zoom") {
            cfg.map.max_zoom = v;
        }
        if let Ok(v) = t.get::<f64>("zoom_step") {
            cfg.map.zoom_step = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("render") {
        if let Ok(v) = t.get::<String>("style") {
            cfg.render.style = v;
        }
        if let Ok(v) = t.get::<String>("language") {
            cfg.render.language = v;
        }
    }
    if let Ok(t) = opt.get::<Table>("cache") {
        if let Ok(v) = t.get::<bool>("tiles") {
            cfg.cache.tiles = v;
        }
        if let Ok(v) = t.get::<usize>("memory_tiles") {
            cfg.cache.memory_tiles = v;
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
    if let Ok(disable) = opt.get::<Table>("disable") {
        cfg.plugins.disable = disable
            .sequence_values::<String>()
            .filter_map(Result::ok)
            .collect();
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

/// Resolve `~/.config/ttymap/init.lua` (or the platform-specific
/// equivalent). `None` only when the host doesn't expose a config
/// dir at all.
fn init_lua_path() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().join("init.lua"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> (Config, KeybindingOverrides) {
        crate::lua::runtimepath::ensure_runtime_path_for_tests();
        let sources = vec![(source.to_string(), PathBuf::from("test"))];
        exec(&sources, &Config::default()).expect("exec init.lua")
    }

    #[test]
    fn empty_init_returns_defaults() {
        let (cfg, km) = run("");
        let d = Config::default();
        assert_eq!(cfg.render.style, d.render.style);
        assert_eq!(cfg.cache.memory_tiles, d.cache.memory_tiles);
        assert!(km.is_empty());
    }

    #[test]
    fn opt_render_style_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.render.style = "bright""#);
        assert_eq!(cfg.render.style, "bright");
    }

    #[test]
    fn opt_cache_memory_tiles_overrides_default() {
        let (cfg, _) = run(r#"ttymap.opt.cache.memory_tiles = 1024"#);
        assert_eq!(cfg.cache.memory_tiles, 1024);
    }

    #[test]
    fn opt_map_zoom_optional_can_be_set() {
        let (cfg, _) = run(r#"ttymap.opt.map.zoom = 10"#);
        assert_eq!(cfg.map.zoom, Some(10.0));
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
        assert_eq!(cfg.cache.memory_tiles, Config::default().cache.memory_tiles);
    }

    #[test]
    fn programmatic_config_works_in_lua() {
        // Conditional / computed values — the killer feature over TOML.
        let src = r#"
            local heavy = true
            ttymap.opt.cache.memory_tiles = heavy and 2048 or 256
        "#;
        let (cfg, _) = run(src);
        assert_eq!(cfg.cache.memory_tiles, 2048);
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

    #[test]
    fn syntax_error_in_one_source_does_not_break_the_chain() {
        // Bad syntax in a layer is caught + logged; the rest of the
        // chain still runs and the surviving Config reflects the
        // valid mutations from the other layers. Mirrors how
        // load_init_lua handles a broken bundled init while letting
        // the user's init.lua take effect.
        crate::lua::runtimepath::ensure_runtime_path_for_tests();
        let sources = vec![
            (
                "this is not lua syntax !!!".to_string(),
                PathBuf::from("bad"),
            ),
            (
                r#"ttymap.opt.render.style = "bright""#.to_string(),
                PathBuf::from("good"),
            ),
        ];
        let (cfg, _km) = exec(&sources, &Config::default()).expect("exec");
        assert_eq!(
            cfg.render.style, "bright",
            "a broken layer must not stop a later layer's mutations"
        );
    }
}
