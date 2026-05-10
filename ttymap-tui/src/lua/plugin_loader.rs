//! Per-plugin Lua chunk runner — drains the [`CaptureSlot`] after a
//! plugin's `require` fires and pushes the captured activation surfaces
//! into the [`PluginRegistry`].
//!
//! Called from the plugin-aware `package.searchers` entry installed by
//! [`crate::lua::vm::install_plugin_searcher`]: when a top-level
//! `require "<name>"` from init.lua hits a `<layer>/plugin/<name>.lua`
//! or `<layer>/plugin/<name>/init.lua` file, the searcher returns a
//! wrapper closure that — when Lua invokes it — calls
//! [`register_one`] with the source. `register_one` runs the chunk
//! via [`crate::lua::bridge::handle::load_chunk`] (which sets the slot's
//! `current_plugin` so `register_*` capturers attribute correctly),
//! drains the captures into the registry, and surfaces help metadata.
//!
//! A plugin is one of two shapes per layer:
//!
//! - **flat file**: `<layer>/plugin/wiki.lua` → registered as `wiki`.
//! - **directory with `init.lua`**: `<layer>/plugin/wiki/init.lua` →
//!   registered as `wiki`. Lets a larger plugin spread its source
//!   across sibling files (`<layer>/plugin/wiki/render.lua`, …)
//!   reachable via `require "wiki.render"` through the stdlib
//!   `package.path` (which still has `<layer>/plugin/?.lua` registered
//!   for sub-module resolution).
//!
//! The init.lua chain (`<bundled>/init.lua` → `~/.config/ttymap/init.lua`)
//! is the only entry point — there is no disk walker. Adding a new
//! bundled plugin is two steps: drop the file under `runtime/plugin/`
//! and add a `require "<name>"` line to `runtime/init.lua`.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use mlua::Lua;

use crate::compositor::{Activation, PaletteEntry, SpawnComponent};
use crate::lua::bridge;
use crate::lua::capture::CaptureSlot;
use crate::lua::host;
use crate::lua::registrar::PluginRegistryHandle;

/// Run one plugin in the shared VM, drain the [`CaptureSlot`] after,
/// and push the captured activation surfaces into `registry`.
///
/// All plugins share `lua` (one Lua VM for the whole subsystem,
/// nvim-style). `slot` is the shared capture slot — drained per
/// plugin so each script's `register_*` calls land in their own
/// bucket attributed to `name`.
pub(super) fn register_one(
    lua: &Lua,
    slot: &CaptureSlot,
    name: &'static str,
    source: &'static str,
    shared: Arc<host::LuaHostShared>,
    registry: &PluginRegistryHandle,
) {
    // Run the script in the shared VM to capture its activation
    // surfaces. We keep clones of `lua` in every closure that fires
    // later so module-level locals (e.g. a `local handle = nil` for
    // toggle-state plugins) survive across the program's lifetime —
    // that's the hook for plugin-side state. All such clones share
    // the underlying `Arc<LuaInner>`.
    let captured = match bridge::handle::load_chunk(lua, source, name, slot) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
            return;
        }
    };

    // `ttymap.on_event(name, fn)` and `ttymap.api.frame.on_tick(fn)`
    // subscribe to the bus directly at call time (each returns an
    // `EventHandle` to the Lua side). They no longer flow through
    // the capture slot; the slot's `events_registered` counter is
    // bumped instead so the load gate sees them.

    // Surface plugin metadata to help. Only entries with a key
    // bind are listed today (matching the prior harvest filter); a
    // plugin with palette commands but no keybind shows up via the
    // palette itself, not via the help cheatsheet.
    if let Some(first_keybind) = captured.keybinds.first() {
        // Pick the activation hint from the first registered
        // keybind. Most plugins declare exactly one; the rare
        // multi-keybind plugin still gets a single help row.
        let key_hint = first_keybind.key.to_string();
        let label = captured
            .palette_commands
            .first()
            .map(|c| c.label.clone())
            .unwrap_or_else(|| name.to_string());
        push_plugin_entry(&shared, name, &key_hint, &label);
    }

    // Explicit-callback paths: each register_palette_command and
    // register_keybind from the script gets its own factory. The
    // factory just runs the captured Lua callback in the shared
    // Lua state. Whatever the callback does — toggle a flag, push
    // a window via `ttymap.api.card.open(spec)`, push a palette
    // via `ttymap.api.palette.open(spec)`, or call a fire-and-forget
    // host API — flows through the channels in `LuaHostHandles` that
    // the App drains every frame. The factory itself never builds
    // or returns a Component; pushing is fully Lua-driven now.
    let build_factory = |gate_key: mlua::RegistryKey, lua_clone: mlua::Lua| -> SpawnComponent {
        std::rc::Rc::new(move |_ctx| {
            run_lua_callback(&lua_clone, &gate_key, name);
            None
        })
    };

    for cmd in captured.palette_commands {
        let factory = build_factory(cmd.invoke, lua.clone());
        registry.borrow_mut().add_palette_entry(
            cmd.id,
            PaletteEntry {
                label: cmd.label,
                hint: cmd.hint,
                name,
                spawn: factory,
            },
        );
    }
    for bind in captured.keybinds {
        let factory = build_factory(bind.callback, lua.clone());
        registry.borrow_mut().add_activation(
            bind.id,
            Activation {
                code: KeyCode::Char(bind.key),
                modifiers: KeyModifiers::NONE,
                spawn: factory,
            },
        );
    }

    // `lua` was cloned into each factory closure (clones share the
    // underlying VM, so any one alive keeps callbacks invokable).
    // The original handle goes out of scope here.
}

/// Run a captured Lua callback (palette command's invoke or
/// keybind's callback). The callback's return value is ignored —
/// the callback drives plugin state through host APIs
/// (`ttymap.api.card.open`, `ttymap.api.palette.open`,
/// `ttymap.map:jump`, …) whose effects flow through the setup
/// state's `LuaHostHandles`. Errors are logged with the plugin's
/// name but don't propagate.
fn run_lua_callback(lua: &Lua, key: &mlua::RegistryKey, name: &'static str) {
    let f: mlua::Function = match lua.registry_value(key) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("lua[{}]: callback registry lookup failed: {}", name, e);
            return;
        }
    };
    if let Err(e) = f.call::<mlua::Value>(()) {
        log::warn!("lua[{}]: callback failed: {}", name, e);
    }
}

/// Surface a plugin's metadata to help via the shared snapshot.
/// Callers gate on a non-empty `key` so the snapshot only carries
/// entries with a top-level keybinding — matching the harvest filter
/// help relied on previously. Overlays don't show up in the palette
/// and aren't help-relevant, so they're never pushed.
fn push_plugin_entry(shared: &Arc<host::LuaHostShared>, name: &str, key: &str, label: &str) {
    shared.push_palette_entry(host::PluginEntry {
        name: name.to_string(),
        key: key.to_string(),
        label: label.to_string(),
    });
}
