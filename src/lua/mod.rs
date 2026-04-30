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
pub mod handle;
pub mod host;
pub mod init_lua;
pub mod map_api;
pub mod palette_provider;
pub mod runtimepath;
pub mod sgp4;

pub use component::LuaComponent;
pub use init_lua::run_init_lua;
pub use palette_provider::LuaPaletteProvider;
pub use runtimepath::{resolve_runtime_path, runtime_path, set_runtime_path};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mlua::{Lua, Table};

use crate::compositor::Registrar;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM with
/// these extras wired in:
///
/// 1. A custom `package.searchers` entry that resolves `require` by
///    reading `<layer>/lua/<name>.lua` from disk, walking every
///    runtime-path layer in priority order — first hit wins, so a
///    user-tier `~/.config/ttymap/lua/ttymap/fmt.lua` shadows the
///    bundled one. Mirrors Neovim's runtime-path searcher.
/// 2. `package.path` extended with each runtime-path layer's `lua/`
///    plus the user plugin dir, so plugins can `require` their
///    filesystem siblings.
///
/// Search order follows Lua's own `package.searchers` precedence: the
/// runtime-libs searcher is appended *after* the standard ones, so a
/// plugin author who puts a `helper.lua` next to their script still
/// wins from `package.path` over a runtime-path collision.
pub fn new_lua() -> Lua {
    let lua = Lua::new();
    if let Err(e) = install_builtin_searcher(&lua) {
        log::warn!("lua: failed to install builtin searcher: {}", e);
    }
    // Higher-priority layers first — `package.path` is searched in
    // order, so the user-tier `lua/` wins over bundled.
    for layer in runtime_path() {
        prepend_package_path(&lua, &layer.join("lua"));
    }
    if let Some(dir) = user_plugins_dir() {
        prepend_package_path(&lua, &dir);
    }
    lua
}

/// Append a `package.searchers` entry that resolves `require "x.y"`
/// by walking [`runtime_path`] and trying `<layer>/lua/x/y.lua` on
/// each layer in priority order. First hit wins.
///
/// When [`runtime_path`] is empty (early test, or runtime resolution
/// failed), the searcher reports a miss for every name and Lua falls
/// through to the standard `package.searchers`.
///
/// The searcher returns:
/// - `function` (the loaded chunk) on hit
/// - `string` (error message) on miss, which Lua appends to the
///   `module 'X' not found:` accumulator before trying the next
///   searcher
fn install_builtin_searcher(lua: &Lua) -> mlua::Result<()> {
    let searcher = lua.create_function(|lua, name: String| -> mlua::Result<mlua::Value> {
        let layers = runtime_path();
        if layers.is_empty() {
            let msg = format!("\n\tno runtime path set, can't resolve '{}'", name);
            return Ok(mlua::Value::String(lua.create_string(&msg)?));
        }
        let rel = name.replace('.', "/");
        let mut tried: Vec<String> = Vec::new();
        for layer in layers {
            let path = layer.join("lua").join(format!("{}.lua", rel));
            match std::fs::read_to_string(&path) {
                Ok(source) => {
                    let chunk = lua.load(source).set_name(&name).into_function()?;
                    return Ok(mlua::Value::Function(chunk));
                }
                Err(_) => tried.push(path.display().to_string()),
            }
        }
        let msg = format!(
            "\n\tno builtin lib '{}' (tried: {})",
            name,
            tried.join(", ")
        );
        Ok(mlua::Value::String(lua.create_string(&msg)?))
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
// Bundled Lua plugins live on disk under `<layer>/lua/*.lua`,
// alongside lib scripts under `<layer>/lua/ttymap/*.lua`. Adding a
// new builtin = drop a `.lua` file under `runtime/lua/` and `make
// install`. There is no Rust array to keep in sync — `register_one`
// reads each file's own metadata (`activation`, `kind`, `key`,
// `label`, `enabled`) to decide how to wire it.
//
// The runtime path itself is discovered at startup via
// [`runtimepath::resolve_runtime_path`]; see that module for the
// resolution order.

/// Register every bundled Lua plugin with the registrar by walking
/// `<layer>/lua/*.lua` for each layer in `runtime_path`, in priority
/// order. Stem dedup means a higher-priority layer's plugin shadows
/// a lower-priority one with the same file name — drop a
/// `~/.config/ttymap/lua/wiki.lua` to replace bundled `wiki`.
///
/// Each script's own metadata drives how it's wired — see
/// [`register_one`] for the activation / kind dispatch.
///
/// Subdirectories (notably `ttymap/` for lib scripts) are skipped by
/// the walker's `.extension() == Some("lua")` filter. Lib scripts are
/// reached via the [`install_builtin_searcher`] hook from `require`,
/// not from this walk.
pub fn register_builtin_plugins(
    runtime_path: &[PathBuf],
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    if runtime_path.is_empty() {
        log::warn!("lua: empty runtime path, no bundled plugins will load");
        return;
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for layer in runtime_path {
        let lua_dir = layer.join("lua");
        if !lua_dir.is_dir() {
            continue;
        }
        register_plugins_in(&lua_dir, Some(&mut seen), shared.clone(), r);
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
    /// Plugin-local key bindings declared as `module.footer_hints`.
    /// Surfaced to help via `ttymap.help:palette_entries()` so the
    /// cheatsheet shows every plugin's local keys, not just its
    /// activation key. Empty when the script omits the field.
    footer_hints: Vec<(String, String)>,
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
    ///
    /// Uses [`new_lua`] (full searcher + package.path setup) rather
    /// than a bare `Lua::new()` because module load can trigger
    /// top-level `require` — `scalebar.lua` does `local fmt = require
    /// "ttymap.fmt"` at the file head, which needs the runtime-path
    /// searcher installed. Without it, the metadata read fails and
    /// the dispatcher falls back to default Toggle/Component, silently
    /// dropping `activation = "overlay"` declarations.
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
                    footer_hints: Vec::new(),
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
        let footer_hints = parse_footer_hints(&module);
        Self {
            activation,
            kind,
            label: label.unwrap_or(default_label),
            key,
            enabled,
            footer_hints,
        }
    }
}

/// Read `module.footer_hints` as owned `(key, label)` pairs.
/// Mirrors the leak-and-static version in [`component::parse_footer_hints`]
/// but returns owned strings so the snapshot in [`host::PluginEntry`]
/// can be rebuilt on each app run without leaking. Two accepted shapes
/// per pair:
/// - `{ "Enter", "open" }` — positional 1-based array.
/// - `{ key = "Enter", label = "open" }` — named.
fn parse_footer_hints(module: &Table) -> Vec<(String, String)> {
    let Ok(list): mlua::Result<Table> = module.get("footer_hints") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in list.sequence_values::<mlua::Value>().flatten() {
        let mlua::Value::Table(pair) = entry else {
            continue;
        };
        let key: String = pair
            .get::<String>("key")
            .or_else(|_| pair.get::<String>(1))
            .unwrap_or_default();
        let label: String = pair
            .get::<String>("label")
            .or_else(|_| pair.get::<String>(2))
            .unwrap_or_default();
        if key.is_empty() && label.is_empty() {
            continue;
        }
        out.push((key, label));
    }
    out
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

/// Wire one Component-shaped Lua module — single-module file or
/// one entry within a multi-entry pack. `entry_idx` is `Some(i)`
/// for the entries path, `None` otherwise; it threads through to
/// the factory closure that re-builds the `LuaComponent` on each
/// activation.
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
            r.add_toggle(label.clone(), key_hint.clone(), name, move |_| {
                component_or_placeholder(name, source, shared_for_toggle.clone())
            });
            if let Some(key) = meta.key {
                use crossterm::event::{KeyCode, KeyModifiers};
                let shared_for_key = shared.clone();
                r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| {
                    component_or_placeholder(name, source, shared_for_key.clone())
                });
            }
            if !key_hint.is_empty() {
                push_plugin_entry(&shared, name, &key_hint, &label, &meta.footer_hints);
            }
        }
        Activation::Spawn => {
            let shared_for_spawn = shared.clone();
            r.add_spawn(label.clone(), key_hint.clone(), name, move |_| {
                component_or_placeholder(name, source, shared_for_spawn.clone())
            });
            if let Some(key) = meta.key {
                use crossterm::event::{KeyCode, KeyModifiers};
                let shared_for_key = shared.clone();
                r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| {
                    component_or_placeholder(name, source, shared_for_key.clone())
                });
            }
            if !key_hint.is_empty() {
                push_plugin_entry(&shared, name, &key_hint, &label, &meta.footer_hints);
            }
        }
    }
}

/// Surface a plugin's metadata to help via the shared snapshot.
/// Callers gate on a non-empty `key` so the snapshot only carries
/// entries with a top-level keybinding — matching the harvest filter
/// help relied on previously. Overlays don't show up in the palette
/// and aren't help-relevant, so they're never pushed.
fn push_plugin_entry(
    shared: &Arc<host::LuaHostShared>,
    name: &str,
    key: &str,
    label: &str,
    footer_hints: &[(String, String)],
) {
    shared.push_palette_entry(host::PluginEntry {
        name: name.to_string(),
        key: key.to_string(),
        label: label.to_string(),
        footer_hints: footer_hints.to_vec(),
    });
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

    r.add_spawn(label.clone(), key_hint.clone(), name, {
        let make = make.clone();
        move |_| make()
    });
    if let Some(key) = meta.key {
        use crossterm::event::{KeyCode, KeyModifiers};
        r.bind(KeyCode::Char(key), KeyModifiers::NONE, move |_| make());
    }
    if !key_hint.is_empty() {
        push_plugin_entry(&shared, name, &key_hint, &label, &meta.footer_hints);
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

/// Scan `~/.config/ttymap/plugins/` for plugin files / dirs and
/// register each. Two layouts are accepted:
/// - flat file: `my.lua` → plugin `my`
/// - directory: `my/init.lua` → plugin `my`, with siblings
///   (`my/state.lua`, …) reachable via `require "my.state"`
///
/// Whether a plugin is *active* is decided by the script itself
/// via the optional `enabled` field on its returned module table
/// — `enabled = false` keeps the file in place but skips
/// registration, which is the natural shape for user-edited
/// scripts (the file *is* the config).
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
    register_plugins_in(&dir, None, shared, r);
}

/// Walk `dir` and route each plugin through the same [`register_one`]
/// dispatcher used by both bundled and user plugins. Two layouts
/// are accepted, both produce the same `<stem>` plugin id:
///
/// - **flat file**: `<dir>/wiki.lua` → id `wiki`.
/// - **directory with `init.lua`**: `<dir>/wiki/init.lua` → id
///   `wiki`. Lets a larger plugin spread its source across sibling
///   files (`<dir>/wiki/render.lua`, `<dir>/wiki/state.lua`, …)
///   reachable via `require "wiki.render"` through the
///   runtime-path searcher and the extended `package.path`. Shared
///   lib namespaces (`<dir>/ttymap/`) are skipped because they
///   don't carry an `init.lua`.
///
/// `seen` is `Some` when the caller is walking multiple layers in
/// priority order and wants stem dedup (a higher-priority layer's
/// `wiki.lua` shadows a lower's). Pass `None` for single-dir walks
/// (the user-plugin dir today) — every file registers regardless of
/// stem collisions across calls.
///
/// Plugins are loaded in alphabetical order of their stem so palette
/// entries surface predictably across runs. When both `wiki.lua` and
/// `wiki/init.lua` exist in the same layer, the file form sorts
/// first and wins via `seen` dedup; the directory form is logged
/// and skipped.
fn register_plugins_in(
    dir: &Path,
    mut seen: Option<&mut std::collections::HashSet<String>>,
    shared: Arc<host::LuaHostShared>,
    r: &mut Registrar,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            log::warn!("lua: read_dir {} failed: {}", dir.display(), e);
            return;
        }
    };

    // Collect (stem, path-to-lua-source) pairs. Either a flat
    // `<stem>.lua` or a `<stem>/init.lua`. Sorting by stem gives a
    // deterministic palette order regardless of filesystem readdir
    // ordering.
    let mut plugins: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("lua") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                plugins.push((stem.to_string(), path));
            }
        } else if path.is_dir() {
            let init = path.join("init.lua");
            if init.is_file()
                && let Some(name) = path.file_name().and_then(|s| s.to_str())
            {
                plugins.push((name.to_string(), init));
            }
        }
    }
    plugins.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    for (stem, path) in plugins {
        if let Some(seen) = seen.as_deref_mut() {
            if !seen.insert(stem.clone()) {
                log::info!(
                    "lua[{}]: shadowed by higher-priority runtime layer, skipping {}",
                    stem,
                    path.display()
                );
                continue;
            }
        }
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
        runtimepath::ensure_runtime_path_for_tests();
        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        let rtp = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime")];
        register_builtin_plugins(&rtp, shared, &mut r);

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
        // contains the plugin's stem (lowercased). `satellite` is the
        // single multi-sat tracker — ISS / Hubble live as in-panel
        // entries inside it, not as separate palette toggles.
        for stem in [
            "aircraft",
            "satellite",
            "quake",
            "wiki",
            "here",
            "export",
            "help",
            "search",
        ] {
            assert!(
                palette.iter().any(|l| l.contains(stem)),
                "expected `{stem}` palette entry, got {palette:?}",
            );
        }
        assert_eq!(overlay_count, 3, "info/scalebar/attribution overlays");
    }

    /// Two `LuaComponent` instances with distinct `name`s coexist
    /// on the compositor stack when toggled in sequence — the
    /// production scenario behind #188. Without `dedup_tag`'s
    /// override this would collapse via TypeId fallback (every
    /// Lua plugin shares `Any::type_id`), the first toggle would
    /// be evicted, and the second would never push.
    #[test]
    fn two_lua_components_coexist_on_compositor_stack() {
        use crate::compositor::Component;
        let shared = host::LuaHostShared::empty();
        let iss = LuaComponent::from_source(
            r#"return { name = "iss", render = function() return {} end }"#,
            "iss",
            shared.clone(),
        )
        .expect("build iss");
        let hubble = LuaComponent::from_source(
            r#"return { name = "hubble", render = function() return {} end }"#,
            "hubble",
            shared,
        )
        .expect("build hubble");
        // Per-instance dedup identity is the script's `name`, not
        // TypeId — the override returns `Some(name)`.
        assert_eq!(iss.dedup_tag(), Some("iss"));
        assert_eq!(hubble.dedup_tag(), Some("hubble"));
        assert_ne!(
            iss.dedup_tag(),
            hubble.dedup_tag(),
            "distinct scripts must declare distinct dedup tags",
        );
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

    #[test]
    fn module_meta_reads_footer_hints_in_both_shapes() {
        let meta = ModuleMeta::parse(
            r#"return {
                name = "x",
                footer_hints = {
                    { "Enter", "open" },
                    { key = "Esc", label = "back" },
                },
            }"#,
            "x",
        );
        assert_eq!(meta.footer_hints.len(), 2);
        assert_eq!(meta.footer_hints[0], ("Enter".into(), "open".into()));
        assert_eq!(meta.footer_hints[1], ("Esc".into(), "back".into()));
    }

    #[test]
    fn registering_a_plugin_publishes_metadata_with_footer_hints() {
        // Round-trip the registration path: a Lua source declaring
        // `key`, `label`, and `footer_hints` should land in the
        // shared snapshot help reads via `ttymap.help:palette_entries()`.
        let dir = temp_plugins_dir("publish-meta");
        write_plugin(
            &dir,
            "demo.lua",
            r#"
            return {
                name = "demo",
                key = "d",
                label = "Toggle demo",
                render = function() return {} end,
                footer_hints = {
                    { "Enter", "open" },
                    { "Esc",   "close" },
                },
            }
            "#,
        );

        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        register_plugins_in(&dir, None, shared.clone(), &mut r);

        let entries = shared.palette_entries.lock().expect("lock palette_entries");
        let demo = entries
            .iter()
            .find(|e| e.name == "demo")
            .expect("demo plugin should be in snapshot");
        assert_eq!(demo.key, "d");
        assert_eq!(demo.label, "Toggle demo");
        assert_eq!(demo.footer_hints.len(), 2);
        assert_eq!(demo.footer_hints[0], ("Enter".into(), "open".into()));
        assert_eq!(demo.footer_hints[1], ("Esc".into(), "close".into()));
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
        let ls = labels(&r);
        assert!(ls.iter().any(|l| l.contains("alpha")));
        assert!(!ls.iter().any(|l| l.contains("beta")), "got {:?}", ls);
    }

    #[test]
    fn dir_discovery_picks_up_plugin_subdirectory_with_init_lua() {
        // `<dir>/wiki/init.lua` registers as plugin `wiki`, mirroring
        // Neovim's plugin directory convention. Lets a large script
        // spread its source across sibling files (state.lua, api.lua,
        // etc.) reachable via `require "wiki.state"` through the
        // extended package.path.
        let dir = temp_plugins_dir("dir-layout");
        std::fs::create_dir_all(dir.join("biggie")).expect("mkdir");
        std::fs::write(
            dir.join("biggie").join("init.lua"),
            r#"return { name = "biggie", render = function() return {} end }"#,
        )
        .expect("write init.lua");

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
        let ls = labels(&r);
        assert!(
            ls.iter().any(|l| l.contains("biggie")),
            "expected biggie/init.lua to register, got {:?}",
            ls
        );
    }

    #[test]
    fn dir_discovery_skips_subdirectory_without_init_lua() {
        // Shared lib namespaces (e.g. `<dir>/ttymap/` carrying
        // `fmt.lua` and similar) are siblings of plugins, not
        // plugins themselves. Without an `init.lua` in the dir,
        // the walker leaves them alone.
        let dir = temp_plugins_dir("dir-no-init");
        std::fs::create_dir_all(dir.join("libonly")).expect("mkdir");
        std::fs::write(
            dir.join("libonly").join("fmt.lua"),
            r#"return { format = function(s) return s end }"#,
        )
        .expect("write fmt.lua");

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
        assert!(
            !labels(&r).iter().any(|l| l.contains("libonly")),
            "lib-only subdir must not register as a plugin"
        );
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
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
        runtimepath::ensure_runtime_path_for_tests();
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
        runtimepath::ensure_runtime_path_for_tests();
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
        register_plugins_in(&dir, None, host::LuaHostShared::empty(), &mut r);
        assert!(r.palette_entries.is_empty());
    }
}
