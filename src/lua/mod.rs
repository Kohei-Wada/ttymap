//! Lua runtime scaffold for scripted plugins.
//!
//! Owns the shared [`mlua::Lua`] state. The bridge surface (Component
//! adapter, MapApi, widget descriptors, etc.) lands in submodules as
//! it gets built out per the audit in `docs/lua-bridge-surface.md`.
//!
//! Defaults that are deliberate and not provisional (see audit §13):
//! - **Lua 5.4** via the `vendored` mlua feature: portable, no
//!   system Lua dep, ~1 MB binary growth.
//! - **No sandbox**: ttymap is single-user; trust the plugin author
//!   (the maintainer themself for now).
//! - **Errors are logged, not propagated**: a buggy plugin must not
//!   crash the host. Helpers in this module wrap mlua results with
//!   `log::warn!` + recovery default.

pub mod component;
pub mod host;
pub mod map_api;

pub use component::LuaComponent;

use std::path::{Path, PathBuf};

use mlua::{Lua, Table};

use crate::compositor::Registrar;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM.
pub fn new_lua() -> Lua {
    Lua::new()
}

/// Bundled `hello` plugin source — a tiny demo that doubles as the
/// reference template for future Lua plugin authors.
const HELLO_LUA: &str = include_str!("scripts/hello.lua");

/// Aircraft plugin (Lua port). Opt-in side-by-side with the Rust
/// version during the migration; once validated the Rust plugin
/// goes away and this becomes the only aircraft implementation.
const AIRCRAFT_LUA: &str = include_str!("scripts/aircraft.lua");

/// Wire the `hello` demo plugin. Called from `app::build_registrar`
/// when `[lua] enabled = true`.
pub fn register_hello(r: &mut Registrar) {
    register_script("hello", HELLO_LUA, r);
}

/// Wire the Lua port of the aircraft plugin. Called from
/// `app::build_registrar` when `[lua_aircraft] enabled = true`.
pub fn register_aircraft(r: &mut Registrar) {
    register_script("aircraft", AIRCRAFT_LUA, r);
}

/// Scan `~/.config/ttymap/plugins/*.lua` and register each as a
/// plugin. The whole point: dropping a `.lua` file in that
/// directory adds a plugin without touching Rust.
///
/// Each file becomes a plugin named after its stem (`my.lua` →
/// `my`). Whether a plugin is *active* is decided by the script
/// itself via the optional `enabled` field on its returned
/// module table — `enabled = false` keeps the file in place but
/// skips registration, which is the natural shape for
/// user-edited scripts (the file *is* the config).
///
/// A read / parse failure on a single file logs a warning and
/// skips it — the rest of the directory still loads. Files are
/// loaded in alphabetical order so palette entries surface in a
/// predictable order across runs.
pub fn register_user_plugins(r: &mut Registrar) {
    let Some(dir) = user_plugins_dir() else {
        // No XDG directory available (no $HOME, weird host) — the
        // user just doesn't get directory-based plugins. Bundled
        // scripts continue to work via their dedicated registrars.
        return;
    };
    if !dir.is_dir() {
        // Directory doesn't exist yet (default case for users who
        // never wrote a plugin). Silent skip — nothing to log.
        return;
    }
    register_user_plugins_from(&dir, r);
}

/// Inner half of [`register_user_plugins`] split out so unit
/// tests can hand a tempdir without faking the XDG layout. Walks
/// `dir`, loads every `*.lua`, and respects each script's own
/// `enabled` flag (default true).
fn register_user_plugins_from(dir: &Path, r: &mut Registrar) {
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            log::warn!("lua: read_dir {} failed: {}", dir.display(), e);
            return;
        }
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    files.sort();
    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("lua: read {} failed: {}", path.display(), e);
                continue;
            }
        };
        if !script_enabled(&source, stem) {
            log::info!(
                "lua[{}]: disabled via module.enabled = false, skipping",
                stem,
            );
            continue;
        }
        // `register_script` requires `&'static str` for the
        // re-load closure that lives for the program lifetime, so
        // the source + name are leaked here. Cost: a few KB per
        // plugin per program lifetime — fine for the ~10 plugins
        // we'll ever have.
        let name: &'static str = Box::leak(stem.to_string().into_boxed_str());
        let source: &'static str = Box::leak(source.into_boxed_str());
        register_script(name, source, r);
    }
}

/// Pre-flight a script just to read its `enabled` field. Anything
/// other than an explicit `false` (parse error, missing module,
/// non-bool value, missing field) is treated as "enabled" — it's
/// safer for the user to see a warning at register time than to
/// have a plugin silently disappear because of a typo.
fn script_enabled(source: &str, name: &str) -> bool {
    let lua = new_lua();
    let module: Table = match lua.load(source).set_name(name).eval() {
        Ok(m) => m,
        Err(_) => return true, // let the real load surface the error
    };
    // mlua coerces nil to `false` for `get::<bool>`, so a missing
    // `enabled` field would *look* like `enabled = false` and turn
    // every plugin off. Read as `Value` and only treat the *explicit*
    // boolean `false` as disabled — nil / missing / wrong type all
    // leave the plugin active so the real load can surface any
    // structural issues.
    !matches!(
        module.get::<mlua::Value>("enabled"),
        Ok(mlua::Value::Boolean(false))
    )
}

/// Resolve `~/.config/ttymap/plugins/` (or the platform-specific
/// equivalent). `None` only when the host doesn't expose a config
/// dir at all — a corner case worth surfacing as "no user plugins"
/// rather than panicking.
fn user_plugins_dir() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().join("plugins"))
}

/// Validate + register one bundled script. Lua failures (parse
/// error, missing fields) are logged and the plugin is silently
/// skipped rather than aborting startup — the host always boots
/// even with a broken Lua plugin.
fn register_script(name: &'static str, source: &'static str, r: &mut Registrar) {
    // Validate the script up front so a syntax error surfaces as one
    // log line instead of a noisy first-toggle failure.
    if let Err(e) = LuaComponent::from_source(source, name) {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }
    let label = format!("Toggle Lua: {}", name);
    r.add_toggle(label, "", move |_| {
        // Re-load on every toggle so the plugin gets fresh state.
        // If it parsed once at startup it should parse again, but
        // recover gracefully if it doesn't.
        LuaComponent::from_source(source, name).unwrap_or_else(|e| {
            log::warn!("lua[{}]: re-load failed: {}", name, e);
            // Synthesize a minimal placeholder so the toggle still
            // produces a Component. Empty source still parses and
            // exposes a no-op render.
            LuaComponent::from_source("return {}", name).expect("trivial Lua module always loads")
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Result;

    #[test]
    fn lua_evaluates_a_basic_expression() {
        let lua = new_lua();
        let n: i64 = lua.load("return 2 + 2").eval().expect("eval 2+2");
        assert_eq!(n, 4);
    }

    #[test]
    fn lua_can_call_a_rust_closure() {
        let lua = new_lua();
        let double = lua
            .create_function(|_, n: i64| Ok(n * 2))
            .expect("create_function");
        lua.globals().set("double", double).expect("set global");
        let n: i64 = lua.load("return double(7)").eval().expect("call");
        assert_eq!(n, 14);
    }

    #[test]
    fn lua_can_return_a_table_to_rust() -> Result<()> {
        let lua = new_lua();
        let table: mlua::Table = lua.load("return { name = 'hi', n = 3 }").eval()?;
        let name: String = table.get("name")?;
        let n: i64 = table.get("n")?;
        assert_eq!(name, "hi");
        assert_eq!(n, 3);
        Ok(())
    }

    #[test]
    fn bundled_hello_script_parses() {
        // The script is in-tree; if this ever fails, the include_str!
        // is pointing at something broken.
        LuaComponent::from_source(HELLO_LUA, "hello").expect("hello.lua should parse");
    }

    #[test]
    fn register_adds_a_palette_entry_for_hello() {
        let mut r = Registrar::default();
        register_hello(&mut r);
        let labels: Vec<&str> = r.palette_entries.iter().map(|e| e.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("hello")),
            "expected a 'hello' palette entry, got {:?}",
            labels,
        );
    }

    #[test]
    fn bundled_aircraft_script_parses() {
        LuaComponent::from_source(AIRCRAFT_LUA, "aircraft").expect("aircraft.lua should parse");
    }

    #[test]
    fn register_aircraft_adds_a_palette_entry() {
        let mut r = Registrar::default();
        register_aircraft(&mut r);
        let labels: Vec<&str> = r.palette_entries.iter().map(|e| e.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("aircraft")),
            "expected an 'aircraft' palette entry, got {:?}",
            labels,
        );
    }

    // ── directory-based discovery ───────────────────────────────

    use std::path::PathBuf;

    /// Build a private temp directory rooted at the OS's temp dir.
    /// `unique` should differ per test so parallel runs don't
    /// stomp on each other.
    fn temp_plugins_dir(unique: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ttymap-lua-test-{}", unique));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn write_plugin(dir: &Path, file_name: &str, lua: &str) {
        std::fs::write(dir.join(file_name), lua).expect("write plugin file");
    }

    fn labels(r: &Registrar) -> Vec<String> {
        r.palette_entries.iter().map(|e| e.label.clone()).collect()
    }

    #[test]
    fn dir_discovery_registers_each_lua_file_under_its_stem() {
        let dir = temp_plugins_dir("registers");
        write_plugin(
            &dir,
            "first.lua",
            r#"return { name = "first", render = function() return {} end }"#,
        );
        write_plugin(
            &dir,
            "second.lua",
            r#"return { name = "second", render = function() return {} end }"#,
        );

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        let ls = labels(&r);
        assert!(ls.iter().any(|l| l.contains("first")), "got {:?}", ls);
        assert!(ls.iter().any(|l| l.contains("second")), "got {:?}", ls);
    }

    #[test]
    fn dir_discovery_skips_non_lua_files() {
        let dir = temp_plugins_dir("skip-non-lua");
        write_plugin(&dir, "ok.lua", r#"return { name = "ok" }"#);
        // README, backup files, etc. should be ignored.
        std::fs::write(dir.join("README.md"), "ignore me").unwrap();
        std::fs::write(dir.join("ok.lua.bak"), "ignore me too").unwrap();

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        let ls = labels(&r);
        assert_eq!(ls.len(), 1, "got {:?}", ls);
        assert!(ls[0].contains("ok"));
    }

    #[test]
    fn dir_discovery_honours_module_enabled_false() {
        let dir = temp_plugins_dir("self-disable");
        write_plugin(&dir, "alpha.lua", r#"return { name = "alpha" }"#);
        // beta opts itself out — the file stays, but the plugin
        // doesn't register.
        write_plugin(
            &dir,
            "beta.lua",
            r#"return { name = "beta", enabled = false }"#,
        );

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        let ls = labels(&r);
        assert!(ls.iter().any(|l| l.contains("alpha")));
        assert!(!ls.iter().any(|l| l.contains("beta")), "got {:?}", ls);
    }

    #[test]
    fn dir_discovery_module_enabled_true_is_explicit_default() {
        // Belt-and-suspenders: a plugin that explicitly sets
        // `enabled = true` registers same as one that omits the
        // field. Guards against accidental tightening of the gate.
        let dir = temp_plugins_dir("self-enable");
        write_plugin(
            &dir,
            "explicit.lua",
            r#"return { name = "explicit", enabled = true }"#,
        );

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        assert!(labels(&r).iter().any(|l| l.contains("explicit")));
    }

    #[test]
    fn dir_discovery_skips_broken_lua_but_keeps_going() {
        let dir = temp_plugins_dir("broken");
        write_plugin(&dir, "broken.lua", "this is not lua syntax !!!");
        write_plugin(
            &dir,
            "ok.lua",
            r#"return { name = "ok", render = function() return {} end }"#,
        );

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        let ls = labels(&r);
        // Broken plugin doesn't make it in; the good one still does.
        assert!(ls.iter().any(|l| l.contains("ok")), "got {:?}", ls);
        assert!(
            !ls.iter().any(|l| l.contains("broken")),
            "broken plugin should not register, got {:?}",
            ls,
        );
    }

    #[test]
    fn dir_discovery_no_op_when_directory_is_missing() {
        // A path that doesn't exist must not panic or error — the
        // common case is "user has never created a plugins/ dir".
        let dir = std::env::temp_dir().join("ttymap-lua-test-missing-xxx-yyy");
        let _ = std::fs::remove_dir_all(&dir);

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, &mut r);
        assert!(r.palette_entries.is_empty());
    }
}
