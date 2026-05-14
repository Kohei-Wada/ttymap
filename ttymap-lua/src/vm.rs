//! Lua VM setup — fresh [`Lua`] state with runtime-path-aware
//! `package.searchers` and `package.path` extensions.
//!
//! [`new_lua`] is the single entry point for building a state. The
//! merged `build_subsystem` calls it once per ttymap process; the
//! same VM hosts init.lua and every `require`-d plugin
//! (nvim-style single state).
//!
//! What this module wires up:
//!
//! - **Lib searcher** (`install_builtin_searcher`, appended after
//!   the stdlib `package.searchers`): walks every runtime layer's
//!   `<layer>/lua/<rel>.lua` for `require "ttymap.fmt"`-style
//!   lookups; returns a plain chunk.
//! - **`package.path` extension** (`prepend_package_path`): adds
//!   each layer's `lua/` to `package.path` so the stdlib
//!   `package.path` searcher resolves the same lib paths.
//!
//! Plugins resolve as **plain Lua modules** under
//! `<layer>/lua/plugin/<name>.lua` — same `package.path` mechanism
//! as any other lib (`ttymap.fmt`, `ttymap.notify`, …). The
//! bundled `runtime/init.lua` activates each by name with
//! `require "plugin.<name>"`. There is no special plugin searcher;
//! "plugin" is just a Lua-side organisational unit (a `.lua` file
//! that calls `register_*`).

use std::path::Path;

use mlua::{Lua, Table};

use crate::runtime_path;

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
/// `<layer>/lua/plugin/<name>.lua` resolves via the same
/// `package.path` mechanism as any other lib — `require
/// "plugin.<name>"` from `runtime/init.lua` Just Works.
pub fn new_lua() -> Lua {
    let lua = Lua::new();
    if let Err(e) = install_builtin_searcher(&lua) {
        log::warn!("lua: failed to install builtin searcher: {}", e);
    }
    // Higher-priority layers first — `package.path` is searched in
    // order, so a user-tier `lua/ttymap/fmt.lua` shadows the bundled
    // one.
    for layer in runtime_path() {
        prepend_package_path(&lua, &layer.join("lua"));
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
