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
pub mod palette_provider;

pub use component::LuaComponent;
pub use palette_provider::LuaPaletteProvider;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mlua::{Lua, Table};

use crate::compositor::Registrar;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM with
/// `package.path` extended so a plugin can `require` its siblings:
///
/// ```lua
/// -- ~/.config/ttymap/plugins/main.lua
/// local utils = require "utils"   -- ~/.config/ttymap/plugins/utils.lua
/// ```
pub fn new_lua() -> Lua {
    let lua = Lua::new();
    if let Some(dir) = user_plugins_dir() {
        prepend_package_path(&lua, &dir);
    }
    lua
}

/// Prepend `<dir>/?.lua` and `<dir>/?/init.lua` to Lua's
/// `package.path` so `require "name"` finds files siblings of the
/// caller in `dir`. Failure is silent — a Lua state without the
/// extra path falls back to the system default, which is fine for
/// plugins that don't `require` anything.
fn prepend_package_path(lua: &Lua, dir: &Path) {
    let Some(dir_str) = dir.to_str() else {
        return;
    };
    let extra = format!("{0}/?.lua;{0}/?/init.lua", dir_str);
    let result: mlua::Result<()> = (|| {
        let package: Table = lua.globals().get("package")?;
        let existing: String = package.get("path")?;
        package.set("path", format!("{};{}", extra, existing))
    })();
    if let Err(e) = result {
        log::warn!("lua: failed to extend package.path with {}: {}", dir_str, e);
    }
}

// ── Builtin scripts ─────────────────────────────────────────────────
//
// Each entry is `(stem, include_str!(...))`. The dispatcher at
// `register_one` reads the script's own metadata (`activation`,
// `kind`, `key`, `label`, `enabled`) to decide how to wire it. Adding
// a new builtin = drop a `.lua` under `scripts/` + 1 line here.

const BUILTIN_SCRIPTS: &[(&str, &str)] = &[
    ("aircraft", include_str!("scripts/aircraft.lua")),
    ("attribution", include_str!("scripts/attribution.lua")),
    ("export", include_str!("scripts/export.lua")),
    ("help", include_str!("scripts/help.lua")),
    ("here", include_str!("scripts/here.lua")),
    ("info", include_str!("scripts/info.lua")),
    ("iss", include_str!("scripts/iss.lua")),
    ("quake", include_str!("scripts/quake.lua")),
    ("scalebar", include_str!("scripts/scalebar.lua")),
    ("search", include_str!("scripts/search.lua")),
    ("wiki", include_str!("scripts/wiki.lua")),
];

/// Register every bundled Lua plugin with the registrar. Each
/// script's own metadata (returned table fields) drives how it's
/// wired — see [`register_one`] for the activation / kind dispatch.
pub fn register_builtin_plugins(shared: Arc<host::LuaHostShared>, r: &mut Registrar) {
    for (name, source) in BUILTIN_SCRIPTS {
        register_one(name, source, shared.clone(), r);
    }
}

/// Plugin shape parsed out of a script's returned table at register
/// time. Drives the dispatcher in [`register_one`].
struct ModuleMeta {
    /// Activation pattern. `"toggle"` (default) installs an
    /// add_toggle palette entry; `"overlay"` installs an always-on
    /// overlay; `"spawn"` installs an add_spawn palette entry (one
    /// new instance per click — used by here/export self-closing
    /// components and by the search palette provider).
    activation: Activation,
    /// Plugin flavour. Components push onto the compositor stack;
    /// providers seed the universal palette picker.
    kind: Kind,
    /// Palette label. Defaults to `"Toggle <name>"` for toggles,
    /// `<name>` for spawns. Empty for overlays (no palette entry).
    label: String,
    /// Optional activation key — for `toggle`/`spawn`, also binds
    /// the key directly so the keybind and palette entry share a
    /// factory.
    key: Option<char>,
    /// Whether the script asked to be skipped (`module.enabled = false`).
    enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activation {
    Toggle,
    Spawn,
    Overlay,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Component,
    Provider,
}

impl ModuleMeta {
    /// Read the plugin's metadata fields by parsing the source once.
    /// Defaults are picked so a minimal `return { name = "x" }` script
    /// works as a toggle Component named "x".
    fn parse(source: &str, name: &str) -> Self {
        let lua = new_lua();
        let module: Table = match lua.load(source).set_name(name).eval() {
            Ok(m) => m,
            // Parse failure is reported by the real load attempt; here
            // we return defaults so the dispatcher proceeds and surfaces
            // the error in one place.
            Err(_) => {
                return Self {
                    activation: Activation::Toggle,
                    kind: Kind::Component,
                    label: format!("Toggle {}", name),
                    key: None,
                    enabled: true,
                };
            }
        };
        let kind = match module.get::<String>("kind").as_deref() {
            Ok("provider") => Kind::Provider,
            _ => Kind::Component,
        };
        let activation_str: Option<String> = module.get("activation").ok();
        let activation = match activation_str.as_deref() {
            Some("overlay") => Activation::Overlay,
            Some("spawn") => Activation::Spawn,
            // Providers default to spawn (each open is a fresh provider).
            None if kind == Kind::Provider => Activation::Spawn,
            _ => Activation::Toggle,
        };
        let label: Option<String> = module.get("label").ok();
        let key: Option<String> = module.get("key").ok();
        let key = key.and_then(|s| s.chars().next());
        let enabled = !matches!(
            module.get::<mlua::Value>("enabled"),
            Ok(mlua::Value::Boolean(false))
        );
        let default_label = match activation {
            Activation::Toggle => format!("Toggle {}", name),
            Activation::Spawn => name.to_string(),
            Activation::Overlay => String::new(),
        };
        Self {
            activation,
            kind,
            label: label.unwrap_or(default_label),
            key,
            enabled,
        }
    }
}

/// Register one Lua script with the registrar by reading its own
/// metadata. The single dispatcher used by both bundled and user
/// plugins — Rust never knows a specific plugin's name.
fn register_one(
    name: &'static str,
    source: &'static str,
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    let meta = ModuleMeta::parse(source, name);
    if !meta.enabled {
        log::info!(
            "lua[{}]: disabled via module.enabled = false, skipping",
            name
        );
        return;
    }

    match meta.kind {
        Kind::Component => register_component(name, source, &meta, shared, r),
        Kind::Provider => register_provider(name, source, &meta, shared, r),
    }
}

fn register_component(
    name: &'static str,
    source: &'static str,
    meta: &ModuleMeta,
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    // Validate up front so a syntax error surfaces as one log line
    // instead of a noisy first-toggle failure.
    if let Err(e) = LuaComponent::from_source_full(source, name, shared.clone()) {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }

    let key_hint = meta.key.map(|c| c.to_string()).unwrap_or_default();
    let label = meta.label.clone();

    match meta.activation {
        Activation::Overlay => {
            let shared_for_factory = shared.clone();
            r.add_overlay(move |_| {
                component_or_placeholder(name, source, shared_for_factory.clone())
            });
        }
        Activation::Toggle => {
            let shared_for_toggle = shared.clone();
            r.add_toggle(label, key_hint, move |_| {
                component_or_placeholder(name, source, shared_for_toggle.clone())
            });
            if let Some(key) = meta.key {
                use crossterm::event::{KeyCode, KeyModifiers};
                let shared_for_key = shared.clone();
                r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| {
                    component_or_placeholder(name, source, shared_for_key.clone())
                });
            }
        }
        Activation::Spawn => {
            let shared_for_spawn = shared.clone();
            r.add_spawn(label, key_hint, move |_| {
                component_or_placeholder(name, source, shared_for_spawn.clone())
            });
            if let Some(key) = meta.key {
                use crossterm::event::{KeyCode, KeyModifiers};
                let shared_for_key = shared.clone();
                r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| {
                    component_or_placeholder(name, source, shared_for_key.clone())
                });
            }
        }
    }
}

fn register_provider(
    name: &'static str,
    source: &'static str,
    meta: &ModuleMeta,
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    if let Err(e) = LuaPaletteProvider::from_source_full(source, name, shared.clone()) {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }

    let key_hint = meta.key.map(|c| c.to_string()).unwrap_or_default();
    let label = meta.label.clone();

    let make = {
        let shared = shared.clone();
        move || -> crate::palette::PaletteComponent {
            let provider = LuaPaletteProvider::from_source_full(source, name, shared.clone())
                .unwrap_or_else(|e| {
                    log::warn!("lua[{}]: re-load failed: {}", name, e);
                    LuaPaletteProvider::from_source_full(
                        "return {}",
                        "lua-fallback",
                        shared.clone(),
                    )
                    .expect("trivial provider always loads")
                });
            crate::palette::PaletteComponent::with_provider(provider)
        }
    };

    r.add_spawn(label, key_hint, {
        let make = make.clone();
        move |_| make()
    });
    if let Some(key) = meta.key {
        use crossterm::event::{KeyCode, KeyModifiers};
        r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| make());
    }
}

/// Try to build a `LuaComponent` from `source`; on failure log + fall
/// back to a no-op module so the host stays alive.
fn component_or_placeholder(
    name: &'static str,
    source: &'static str,
    shared: Arc<host::LuaHostShared>,
) -> LuaComponent {
    LuaComponent::from_source_full(source, name, shared.clone()).unwrap_or_else(|e| {
        log::warn!("lua[{}]: re-load failed: {}", name, e);
        LuaComponent::from_source_full("return {}", name, shared)
            .expect("trivial Lua module always loads")
    })
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
pub fn register_user_plugins(shared: Arc<host::LuaHostShared>, r: &mut Registrar) {
    let Some(dir) = user_plugins_dir() else {
        return;
    };
    if !dir.is_dir() {
        return;
    }
    register_user_plugins_from(&dir, shared, r);
}

/// Inner half of [`register_user_plugins`] split out so unit
/// tests can hand a tempdir without faking the XDG layout. Walks
/// `dir`, loads every `*.lua`, and routes each through the same
/// [`register_one`] dispatcher used for bundled plugins.
fn register_user_plugins_from(dir: &Path, shared: Arc<host::LuaHostShared>, r: &mut Registrar) {
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
        // `register_one` requires `&'static str` for the re-load
        // closure that lives for the program lifetime; leak both.
        // Cost: a few KB per plugin per program lifetime.
        let name: &'static str = Box::leak(stem.to_string().into_boxed_str());
        let source: &'static str = Box::leak(source.into_boxed_str());
        register_one(name, source, shared.clone(), r);
    }
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

/// Validate + register one bundled script as a palette toggle.
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

    /// All bundled scripts must parse cleanly. Subscript- and
    /// activation-specific behaviour is covered by the unified
    /// dispatcher tests below.
    #[test]
    fn every_bundled_script_parses() {
        for (name, source) in BUILTIN_SCRIPTS {
            // Some scripts (search) are providers, not components —
            // detect via `kind = "provider"` and dispatch.
            let kind_is_provider = source.contains(r#"kind = "provider""#);
            if kind_is_provider {
                LuaPaletteProvider::from_source(source, name)
                    .unwrap_or_else(|e| panic!("{}.lua should parse: {}", name, e));
            } else {
                LuaComponent::from_source(source, name)
                    .unwrap_or_else(|e| panic!("{}.lua should parse: {}", name, e));
            }
        }
    }

    /// `register_builtin_plugins` should install every script. The
    /// exact counts are: 3 overlays (info, scalebar, attribution),
    /// the rest land as palette entries (some with key binds). A
    /// regression that drops any single plugin would shift these.
    #[test]
    fn register_builtin_plugins_installs_every_bundled_script() {
        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        register_builtin_plugins(shared, &mut r);

        // Exactly 3 always-on overlays today: info, scalebar, attribution.
        assert_eq!(r.overlays.len(), 3, "overlays count");

        // Bundled palette entries: aircraft, iss, quake, wiki, here,
        // export, help, search → 8.
        assert_eq!(r.palette_entries.len(), 8, "palette entries count");

        // Key binds: wiki ('i'), help ('?'), search ('/') → 3.
        assert_eq!(r.activations.len(), 3, "activation key binds count");
    }

    /// Module metadata: defaults, overrides, kind/activation flips.
    #[test]
    fn module_meta_defaults_are_toggle_component() {
        let meta = ModuleMeta::parse(r#"return { name = "x" }"#, "x");
        assert!(matches!(meta.activation, Activation::Toggle));
        assert!(matches!(meta.kind, Kind::Component));
        assert_eq!(meta.label, "Toggle x");
        assert!(meta.key.is_none());
        assert!(meta.enabled);
    }

    #[test]
    fn module_meta_picks_up_overlay_activation() {
        let meta = ModuleMeta::parse(r#"return { name = "x", activation = "overlay" }"#, "x");
        assert!(matches!(meta.activation, Activation::Overlay));
    }

    #[test]
    fn module_meta_provider_default_activation_is_spawn() {
        let meta = ModuleMeta::parse(r#"return { name = "x", kind = "provider" }"#, "x");
        assert!(matches!(meta.kind, Kind::Provider));
        assert!(matches!(meta.activation, Activation::Spawn));
    }

    #[test]
    fn module_meta_reads_key_label_enabled() {
        let meta = ModuleMeta::parse(
            r#"return { name = "x", key = "i", label = "Toggle x", enabled = false }"#,
            "x",
        );
        assert_eq!(meta.key, Some('i'));
        assert_eq!(meta.label, "Toggle x");
        assert!(!meta.enabled);
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
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
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
    fn package_path_extension_lets_require_find_siblings() {
        // Drop a `helper.lua` into a tempdir, point Lua's
        // `package.path` at it, then `require "helper"` from a
        // plain Lua chunk. The require must resolve to the file
        // we wrote — proves the extension is wired.
        let dir = temp_plugins_dir("require");
        std::fs::write(
            dir.join("helper.lua"),
            "return { greet = function(name) return 'hi ' .. name end }",
        )
        .unwrap();

        let lua = Lua::new();
        prepend_package_path(&lua, &dir);
        let greeting: String = lua
            .load(r#"return require("helper").greet("world")"#)
            .eval()
            .expect("require should find helper.lua via prepended package.path");
        assert_eq!(greeting, "hi world");
    }

    #[test]
    fn package_path_unchanged_for_unrelated_modules() {
        // The extension only adds a *prefix*; existing entries
        // (system Lua paths, mlua's defaults) stay reachable, so
        // a `require "doesnotexist"` still produces the standard
        // "module not found" error rather than something exotic.
        let dir = temp_plugins_dir("require-passthrough");
        let lua = Lua::new();
        prepend_package_path(&lua, &dir);
        let res: mlua::Result<i64> = lua.load(r#"return require("doesnotexist")"#).eval();
        assert!(res.is_err(), "non-existent require should still error");
    }

    #[test]
    fn dir_discovery_no_op_when_directory_is_missing() {
        // A path that doesn't exist must not panic or error — the
        // common case is "user has never created a plugins/ dir".
        let dir = std::env::temp_dir().join("ttymap-lua-test-missing-xxx-yyy");
        let _ = std::fs::remove_dir_all(&dir);

        let mut r = Registrar::default();
        register_user_plugins_from(&dir, host::LuaHostShared::empty(), &mut r);
        assert!(r.palette_entries.is_empty());
    }
}
