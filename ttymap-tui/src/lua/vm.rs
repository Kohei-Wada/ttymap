//! Lua VM setup — fresh [`Lua`] state with runtime-path-aware
//! `package.searchers` and `package.path` extensions.
//!
//! [`new_lua`] is the single entry point for building a state. The
//! merged `build_subsystem` calls it once per ttymap process; the
//! same VM hosts init.lua and every `require`-d plugin
//! (nvim-style single state).
//!
//! Two searchers cooperate in `package.searchers`:
//!
//! 1. **Plugin searcher** (inserted at position 2 by
//!    [`install_plugin_searcher`], so it fires *before* `package.path`).
//!    Owns **all** `<layer>/plugin/...` resolution. Top-level requires
//!    (no dot in `name`) hit the plugin entry (`plugin/<name>.lua` or
//!    `plugin/<name>/init.lua`) and run through
//!    [`super::plugin_loader::register_one`] so registrations land in
//!    the [`PluginRegistry`] under `<name>`. Dotted requires
//!    (`require "travel.routes.italy"` from inside `plugin/travel/init.lua`)
//!    are sub-modules: same path resolution
//!    (`plugin/travel/routes/italy.lua`), but plain chunk — no
//!    attribution wrap.
//! 2. **Lib searcher** (appended after the stdlib ones by
//!    [`install_builtin_searcher`]). Walks `<layer>/lua/<rel>.lua` for
//!    `require "ttymap.fmt"`-style lib lookups; returns a plain chunk.
//!
//! `package.path` is extended with each layer's `lua/` only — the
//! `plugin/` directory is exclusively the plugin searcher's domain,
//! so the two trees stay cleanly separated (libs in `lua/`, plugins
//! and their internal sub-modules in `plugin/<plugin>/...`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mlua::{Function, Lua, Table};

use crate::lua::capture::CaptureSlot;
use crate::lua::host::LuaHostShared;
use crate::lua::registrar::PluginRegistryHandle;
use crate::lua::runtime_path;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM with
/// these extras wired in:
///
/// 1. A custom `package.searchers` entry that resolves `require` by
///    reading `<layer>/lua/<name>.lua` from disk, walking every
///    runtime-path layer in priority order — first hit wins, so a
///    user-tier `~/.config/ttymap/lua/ttymap/fmt.lua` shadows the
///    bundled one. Mirrors Neovim's runtime-path searcher.
/// 2. `package.path` extended with each runtime-path layer's `lua/`,
///    so the stdlib `package.path` searcher resolves the same lib
///    paths.
///
/// `<layer>/plugin/...` is **not** added to `package.path` — that
/// tree is owned exclusively by [`install_plugin_searcher`], which
/// handles both top-level plugin requires (with attribution wrap)
/// and dotted sub-module requires (plain chunk). Keeps the two
/// trees cleanly separated.
pub fn new_lua() -> Lua {
    let lua = Lua::new();
    if let Err(e) = install_builtin_searcher(&lua) {
        log::warn!("lua: failed to install builtin searcher: {}", e);
    }
    // Higher-priority layers first — `package.path` is searched in
    // order, so a user-tier `lua/ttymap/fmt.lua` shadows the bundled
    // one. Plugins live under `<layer>/plugin/` and are resolved by
    // the plugin searcher (see `install_plugin_searcher`).
    for layer in runtime_path() {
        prepend_package_path(&lua, &layer.join("lua"));
    }
    lua
}

/// Install the plugin-aware searcher at position 2 in
/// `package.searchers` (after `preload`, before the stdlib
/// `package.path` searcher). Owns **all** `<layer>/plugin/...`
/// resolution; `package.path` no longer carries `plugin/` entries.
///
/// Two cases by require-name shape (path resolution is the same:
/// `name.replace('.', '/')` joined under `<layer>/plugin/`, with a
/// `.lua` and a `/init.lua` candidate):
///
/// - **Top-level** (`require "travel"`): the plugin entry. Returns a
///   wrapper closure that calls [`super::plugin_loader::register_one`]
///   when invoked, attributing every `register_palette_command` /
///   `register_keybind` / `on_event` to `<name>` in the registry.
///   The wrapper returns `Value::Nil`; Lua then sets
///   `package.loaded[name] = true`, so a duplicate require from user
///   init.lua becomes a no-op (registrations don't double-fire).
/// - **Dotted** (`require "travel.routes.italy"` from inside
///   `plugin/travel/init.lua`): a plugin sub-module. Plain chunk —
///   no attribution wrap, no `package.loaded` magic beyond Lua's
///   built-in module caching.
///
/// On miss, returns the standard searcher-protocol "no plugin
/// found" string so Lua falls through to `package.path` (which may
/// resolve a lib at `<layer>/lua/...`) and beyond.
///
/// Caller must ensure the plugin runtime API surface
/// (`ttymap.register_*`, `ttymap.on_event`, …) is already installed
/// before any plugin loads — `register_one` calls into capturers
/// that read from the `ttymap` global.
pub fn install_plugin_searcher(
    lua: &Lua,
    layers: Vec<PathBuf>,
    slot: CaptureSlot,
    registry: PluginRegistryHandle,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<()> {
    let lua_for_capture = lua.clone();
    let searcher = lua.create_function(move |lua, name: String| -> mlua::Result<mlua::Value> {
        if layers.is_empty() {
            let msg = format!("\n\tno runtime path set, can't resolve '{}'", name);
            return Ok(mlua::Value::String(lua.create_string(&msg)?));
        }
        let rel = name.replace('.', "/");
        let is_top_level = !name.contains('.');
        let mut tried: Vec<String> = Vec::new();
        for layer in &layers {
            let candidates = [
                layer.join("plugin").join(format!("{}.lua", rel)),
                layer.join("plugin").join(&rel).join("init.lua"),
            ];
            for cand in candidates {
                match std::fs::read_to_string(&cand) {
                    Ok(source) => {
                        if is_top_level {
                            // Plugin entry — wrap with register_one so
                            // captures attribute under `<name>`.
                            // `register_one` needs `&'static str`
                            // (factory closures live for program
                            // lifetime). Cost: a few KB per plugin
                            // per program lifetime.
                            let leaked_name: &'static str =
                                Box::leak(name.clone().into_boxed_str());
                            let leaked_src: &'static str = Box::leak(source.into_boxed_str());
                            let lua_clone = lua_for_capture.clone();
                            let slot = slot.clone();
                            let registry = registry.clone();
                            let shared = shared.clone();
                            let wrapper = lua.create_function(
                                move |_, _: ()| -> mlua::Result<mlua::Value> {
                                    super::plugin_loader::register_one(
                                        &lua_clone,
                                        &slot,
                                        leaked_name,
                                        leaked_src,
                                        shared.clone(),
                                        &registry,
                                    );
                                    Ok(mlua::Value::Nil)
                                },
                            )?;
                            return Ok(mlua::Value::Function(wrapper));
                        } else {
                            // Sub-module — plain chunk, no attribution.
                            // `set_name(&name)` keeps Lua errors in
                            // the script readable.
                            let chunk = lua.load(source).set_name(&name).into_function()?;
                            return Ok(mlua::Value::Function(chunk));
                        }
                    }
                    Err(_) => tried.push(cand.display().to_string()),
                }
            }
        }
        let msg = format!("\n\tno plugin '{}' (tried: {})", name, tried.join(", "));
        Ok(mlua::Value::String(lua.create_string(&msg)?))
    })?;

    // Insert at position 2 so it runs before the `package.path`
    // searcher (which would otherwise resolve plugin paths as plain
    // chunks via the `<layer>/plugin/?.lua` entries we leave there
    // for sub-module requires). Use Lua's `table.insert` so the
    // existing entries shift up by one.
    let package: Table = lua.globals().get("package")?;
    let searchers: Table = package.get("searchers")?;
    let table_global: Table = lua.globals().get("table")?;
    let insert: Function = table_global.get("insert")?;
    insert.call::<()>((searchers, 2, searcher))?;
    Ok(())
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
pub(super) fn install_builtin_searcher(lua: &Lua) -> mlua::Result<()> {
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
pub(super) fn prepend_package_path(lua: &Lua, dir: &Path) {
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
