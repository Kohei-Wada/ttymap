//! Bundled plugin discovery — walk every `<layer>/plugin/` directory
//! on the runtime path, parse the script's `register_*` /
//! `on_event` declarations via [`crate::lua::bridge::handle::fresh_load`],
//! and feed the resulting activations / palette entries / event-bus
//! subscriptions into the [`Registrar`].
//!
//! nvim-style two-tier layout per runtime layer:
//!
//! - `<layer>/plugin/*.lua` — auto-discovered plugins. The script's
//!   existence is the registration; identity = file stem. The script
//!   subscribes to host loops via `ttymap.api.frame.on_tick(fn)` /
//!   `register_palette_command` / `register_keybind`.
//! - `<layer>/lua/<name>.lua` — `require`-able lib scripts. NOT
//!   auto-discovered. Plugins reach them via `require "<name>"`
//!   through the searcher installed by `lua/vm.rs`.
//!
//! Adding a new builtin = drop a `.lua` file under `runtime/plugin/`
//! and `make install`. There is no Rust array to keep in sync.
//!
//! The runtime path itself is discovered at startup via
//! [`crate::lua::runtimepath::resolve_runtime_path`]; see that module
//! for the resolution order.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use mlua::Lua;

use crate::compositor::op;
use crate::compositor::{Activation, PaletteEntry, SpawnComponent};
use crate::lua::bridge;
use crate::lua::host;
use crate::lua::registrar::Registrar;

/// Register every bundled Lua plugin with the registrar by walking
/// `<layer>/plugin/*.lua` for each layer in `runtime_path`, in
/// priority order. Stem dedup means a higher-priority layer's plugin
/// shadows a lower-priority one with the same file name — drop a
/// `~/.config/ttymap/plugin/wiki.lua` to replace bundled `wiki`.
///
/// `disable` is the user-supplied opt-out list (`ttymap.opt.disable`).
/// A plugin whose stem matches any entry is skipped at registration
/// time.
pub fn register_builtin_plugins(
    runtime_path: &[PathBuf],
    disable: &[String],
    shared: Arc<host::LuaHostShared>,
    ops: op::OpsBuffer,
    r: &mut Registrar,
) {
    if runtime_path.is_empty() {
        log::warn!("lua: empty runtime path, no bundled plugins will load");
        return;
    }
    let mut seen: HashSet<String> = HashSet::new();
    for layer in runtime_path {
        let plugin_dir = layer.join("plugin");
        if !plugin_dir.is_dir() {
            continue;
        }
        register_plugins_in(
            &plugin_dir,
            Some(&mut seen),
            disable,
            shared.clone(),
            ops.clone(),
            r,
        );
    }
}

/// Register one Lua script with the registrar by reading its own
/// subscriptions. The single dispatcher used by both bundled and
/// user plugins — Rust never knows a specific plugin's name; the
/// caller passes the file stem as `name`.
pub(super) fn register_one(
    name: &'static str,
    source: &'static str,
    shared: Arc<host::LuaHostShared>,
    ops: op::OpsBuffer,
    r: &mut Registrar,
) {
    // Run the script once to capture its activation surfaces and
    // tick subscriptions. The `lua` returned here is the **setup
    // state**: it holds the module-level Lua locals from setup, plus
    // the RegistryKey'd palette / keybind / tick callbacks. We keep
    // clones of it in every closure that fires later so module-level
    // vars (e.g. an `enabled` flag) survive across the program's
    // lifetime — that's the hook for plugin-side toggle state.
    let shared_for_plugin = shared.clone();
    // `name` is passed twice: as `chunk_name` (Lua stack-trace label)
    // and as `host_tag` (HTTP UA suffix, log target, fallback window
    // display name). Same value — the file stem is the plugin's
    // canonical identifier on every surface.
    let (lua, captured, handles) =
        match bridge::handle::fresh_load(source, name, name, shared_for_plugin, ops) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
                return;
            }
        };

    // Every `ttymap.on_event(name, fn)` (and its sugar
    // `ttymap.api.frame.on_tick(fn)` which lowers to event "tick")
    // capture lands on the bus as a Lua subscriber. The setup-state
    // Lua is cloned (cheap Arc bump) into each entry so the callback
    // stays invokable for the program's lifetime. Order =
    // registration order.
    for sub in captured.event_subscriptions {
        r.event_bus
            .subscribe_lua(sub.event_name, name, lua.clone(), sub.callback);
    }

    // Hand the setup state's `LuaHostHandles` over to the
    // registrar so the App can drain its receivers per frame.
    // Setup-state callbacks (palette command invoke, register_keybind
    // callback, plugin-level `loop`, and any `ttymap.api.card.open`
    // / `palette.open` spec callbacks) flip these senders. Without
    // this push the receivers would just sit (latent bug pre-A7).
    r.lua_host_handles.push(handles);

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
    // factory just runs the captured Lua callback in the persistent
    // setup state. Whatever the callback does — toggle a flag, push
    // a window via `ttymap.api.card.open(spec)`, push a palette
    // via `ttymap.api.palette.open(spec)`, or call a fire-and-forget
    // host API — flows through the channels in `LuaHostHandles` that
    // the App drains every frame. The factory itself never builds
    // or returns a Component; pushing is fully Lua-driven now.
    let build_factory = |gate_key: mlua::RegistryKey, lua_clone: mlua::Lua| -> SpawnComponent {
        Box::new(move |_ctx| {
            run_lua_callback(&lua_clone, &gate_key, name);
            None
        })
    };

    for cmd in captured.palette_commands {
        let factory = build_factory(cmd.invoke, lua.clone());
        r.palette_entries.push(PaletteEntry {
            label: cmd.label,
            hint: cmd.hint,
            name,
            spawn: factory,
        });
    }
    for bind in captured.keybinds {
        let factory = build_factory(bind.callback, lua.clone());
        r.activations.push(Activation {
            code: KeyCode::Char(bind.key),
            modifiers: KeyModifiers::NONE,
            spawn: factory,
        });
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
pub(super) fn register_plugins_in(
    dir: &Path,
    mut seen: Option<&mut HashSet<String>>,
    disable: &[String],
    shared: Arc<host::LuaHostShared>,
    ops: op::OpsBuffer,
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
        if let Some(seen) = seen.as_deref_mut()
            && !seen.insert(stem.clone())
        {
            log::info!(
                "lua[{}]: shadowed by higher-priority runtime layer, skipping {}",
                stem,
                path.display()
            );
            continue;
        }
        if disable.iter().any(|d| d == &stem) {
            log::info!("lua[{}]: disabled via ttymap.opt.disable, skipping", stem);
            continue;
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
        register_one(name, source, shared.clone(), ops.clone(), r);
    }
}
