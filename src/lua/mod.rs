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
pub mod runtimepath;

pub use component::LuaComponent;
pub use palette_provider::LuaPaletteProvider;
pub use runtimepath::{resolve_runtime_dir, runtime_dir, set_runtime_dir};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mlua::{Lua, Table};

use crate::compositor::Registrar;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM with two
/// extras wired in:
///
/// 1. A custom `package.searchers` entry that resolves `require` by
///    reading `<runtime>/lua/<name>.lua` from disk, so bundled and
///    user plugins can share helpers via `require "ttymap.fmt"`.
///    Mirrors Neovim's runtime-path searcher.
/// 2. `package.path` extended with the user plugin directory and
///    `<runtime>/lua/` so plugins can `require` their filesystem
///    siblings:
///
/// ```lua
/// -- ~/.config/ttymap/plugins/main.lua
/// local utils = require "utils"   -- ~/.config/ttymap/plugins/utils.lua
/// ```
///
/// Search order follows Lua's own `package.searchers` precedence: the
/// runtime-libs searcher is appended *after* the standard ones, so a
/// user plugin file with the same name as a bundled lib still wins
/// from the filesystem. Conflicts are unlikely in practice (the
/// bundled namespace is `ttymap.*`).
pub fn new_lua() -> Lua {
    let lua = Lua::new();
    if let Err(e) = install_builtin_searcher(&lua) {
        log::warn!("lua: failed to install builtin searcher: {}", e);
    }
    if let Some(rt) = runtime_dir() {
        prepend_package_path(&lua, &rt.join("lua"));
    }
    if let Some(dir) = user_plugins_dir() {
        prepend_package_path(&lua, &dir);
    }
    lua
}

/// Append a `package.searchers` entry that resolves `require "x.y"`
/// to `<runtime>/lua/x/y.lua` on disk. Mirrors Neovim's runtime-path
/// searcher: any `.lua` file under the runtime dir is reachable via
/// `require`, with `.` in the module name expanding to `/` in the
/// path.
///
/// When [`runtime_dir`] is unset (early test, or runtime resolution
/// failed), the searcher reports a miss for every name and Lua falls
/// through to the standard `package.searchers`.
///
/// The searcher returns:
/// - `function` (the loaded chunk) on hit, signalling Lua to use it
/// - `string` (error message) on miss, which Lua appends to the
///   `module 'X' not found:` accumulator before trying the next
///   searcher
fn install_builtin_searcher(lua: &Lua) -> mlua::Result<()> {
    let searcher = lua.create_function(|lua, name: String| -> mlua::Result<mlua::Value> {
        let Some(rt) = runtime_dir() else {
            let msg = format!("\n\tno runtime dir set, can't resolve '{}'", name);
            return Ok(mlua::Value::String(lua.create_string(&msg)?));
        };
        let rel = name.replace('.', "/");
        let path = rt.join("lua").join(format!("{}.lua", rel));
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                let chunk = lua.load(source).set_name(&name).into_function()?;
                Ok(mlua::Value::Function(chunk))
            }
            Err(_) => {
                let msg = format!(
                    "\n\tno builtin lib '{}' (looked at {})",
                    name,
                    path.display()
                );
                Ok(mlua::Value::String(lua.create_string(&msg)?))
            }
        }
    })?;
    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;
    let len = searchers.len()?;
    searchers.set(len + 1, searcher)?;
    Ok(())
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

// ── Bundled plugin discovery ────────────────────────────────────────
//
// Bundled Lua plugins live on disk under `<runtime>/lua/*.lua`,
// alongside lib scripts under `<runtime>/lua/ttymap/*.lua`. Adding a
// new builtin = drop a `.lua` file under `runtime/lua/` and `make
// install`. There is no Rust array to keep in sync — `register_one`
// reads each file's own metadata (`activation`, `kind`, `key`,
// `label`, `enabled`) to decide how to wire it.
//
// The runtime dir itself is discovered at startup via
// [`runtimepath::resolve_runtime_dir`]; see that module for the
// resolution order.

/// Register every bundled Lua plugin with the registrar by walking
/// `<runtime_dir>/lua/*.lua`. Each script's own metadata drives how
/// it's wired — see [`register_one`] for the activation / kind
/// dispatch.
///
/// Subdirectories (notably `ttymap/` for lib scripts) are skipped:
/// the walker filters by `.extension() == Some("lua")` which excludes
/// directory entries. Lib scripts are reached via the
/// [`install_builtin_searcher`] hook from `require`, not from this
/// walk.
pub fn register_builtin_plugins(
    runtime_dir: &Path,
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    let lua_dir = runtime_dir.join("lua");
    if !lua_dir.is_dir() {
        log::warn!(
            "lua: runtime dir {} has no `lua/` subdir, no bundled plugins will load",
            runtime_dir.display()
        );
        return;
    }
    register_plugins_in(&lua_dir, shared, r);
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
        // Presence of a `palette` sub-table is the only signal that
        // the script wants palette-provider semantics. There is no
        // separate `kind` field — the shape *is* the declaration.
        let kind = if matches!(
            module.get::<mlua::Value>("palette"),
            Ok(mlua::Value::Table(_))
        ) {
            Kind::Provider
        } else {
            Kind::Component
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
    if let Err(e) = LuaComponent::from_source(source, name, shared.clone()) {
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
            r.add_toggle(label, key_hint, name, move |_| {
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
            r.add_spawn(label, key_hint, name, move |_| {
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
    if let Err(e) = LuaPaletteProvider::from_source(source, name, shared.clone()) {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }

    let key_hint = meta.key.map(|c| c.to_string()).unwrap_or_default();
    let label = meta.label.clone();

    let make = {
        let shared = shared.clone();
        move || -> crate::palette::PaletteComponent {
            let provider = LuaPaletteProvider::from_source(source, name, shared.clone())
                .unwrap_or_else(|e| {
                    log::warn!("lua[{}]: re-load failed: {}", name, e);
                    LuaPaletteProvider::from_source("return {}", "lua-fallback", shared.clone())
                        .expect("trivial provider always loads")
                });
            crate::palette::PaletteComponent::with_provider(provider)
        }
    };

    r.add_spawn(label, key_hint, name, {
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
    LuaComponent::from_source(source, name, shared.clone()).unwrap_or_else(|e| {
        log::warn!("lua[{}]: re-load failed: {}", name, e);
        LuaComponent::from_source("return {}", name, shared)
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
    register_plugins_in(&dir, shared, r);
}

/// Walk `dir`, load every `*.lua`, and route each through the same
/// [`register_one`] dispatcher used for both bundled and user plugins.
/// The single shared entry point — bundled vs user differs only in
/// *which* directory you pass, not in how each file is parsed or
/// wired.
///
/// Subdirectories (e.g. `ttymap/` under `runtime/lua/`) have no
/// `.lua` extension and are filtered out naturally. Files are loaded
/// in alphabetical order so palette entries surface predictably
/// across runs.
fn register_plugins_in(dir: &Path, shared: Arc<host::LuaHostShared>, r: &mut Registrar) {
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

    /// Every bundled script must register cleanly through the same
    /// dispatcher production uses. Asserts each plugin shows up in
    /// some registrar slot (overlay / palette / key bind) — the
    /// dispatcher itself decides which based on `module.activation`,
    /// so this round-trips the parse + meta + wire path. Set-
    /// membership rather than counts so adding a builtin doesn't
    /// require updating magic numbers.
    #[test]
    fn every_bundled_script_registers() {
        runtimepath::ensure_runtime_dir_for_tests();
        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        let rt = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
        register_builtin_plugins(&rt, shared, &mut r);

        let palette: std::collections::HashSet<String> = r
            .palette_entries
            .iter()
            .map(|e| e.label.to_lowercase())
            .collect();
        // `r.overlays` doesn't carry a name; we sanity-check the
        // count of always-on overlays separately (info / scalebar /
        // attribution are the only three today; any builtin
        // declaring `activation = "overlay"` would trip this).
        let overlay_count = r.overlays.len();

        // Toggles + spawns: each leaves a palette entry whose label
        // contains the plugin's stem (lowercased).
        for stem in [
            "aircraft", "iss", "quake", "wiki", "here", "export", "help", "search",
        ] {
            assert!(
                palette.iter().any(|l| l.contains(stem)),
                "expected `{stem}` palette entry, got {palette:?}",
            );
        }
        assert_eq!(overlay_count, 3, "info/scalebar/attribution overlays");
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
    fn palette_subtable_marks_module_as_provider() {
        let meta = ModuleMeta::parse(r#"return { name = "x", palette = {} }"#, "x");
        assert!(matches!(meta.kind, Kind::Provider));
        // Providers default to spawn (each open is a fresh provider).
        assert!(matches!(meta.activation, Activation::Spawn));
    }

    #[test]
    fn no_palette_subtable_means_component() {
        let meta = ModuleMeta::parse(r#"return { name = "x" }"#, "x");
        assert!(matches!(meta.kind, Kind::Component));
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
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
    fn builtin_searcher_resolves_ttymap_fmt() {
        // Bundled lib script `ttymap.fmt` must be reachable via
        // `require` once the searcher is installed — proves the disk-
        // backed runtime-path searcher is wired into package.searchers.
        runtimepath::ensure_runtime_dir_for_tests();
        let lua = Lua::new();
        install_builtin_searcher(&lua).expect("install searcher");
        let out: String = lua
            .load(r#"return require("ttymap.fmt").distance(1500)"#)
            .eval()
            .expect("require ttymap.fmt should succeed");
        assert_eq!(out, "1.5km");
    }

    #[test]
    fn builtin_searcher_misses_fall_through_to_standard_searchers() {
        // The custom searcher must signal "no match" with a string
        // (Lua's searcher protocol) rather than throwing, so unknown
        // requires still hit the standard "module not found" path.
        runtimepath::ensure_runtime_dir_for_tests();
        let lua = Lua::new();
        install_builtin_searcher(&lua).expect("install searcher");
        let res: mlua::Result<i64> = lua.load(r#"return require("nope.nope")"#).eval();
        assert!(res.is_err(), "unknown require should error normally");
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
        register_plugins_in(&dir, host::LuaHostShared::empty(), &mut r);
        assert!(r.palette_entries.is_empty());
    }
}
