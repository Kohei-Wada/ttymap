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

pub mod bridge;
pub mod init_lua;
pub mod registry;
pub mod runtimepath;
pub mod ttymap;

pub use bridge::component::LuaComponent;
pub use bridge::palette_provider::LuaPaletteProvider;
pub use init_lua::run_init_lua;
pub use registry::LuaPluginRegistry;
pub use runtimepath::{resolve_runtime_path, runtime_path, set_runtime_path};
pub use ttymap::LuaHostShared;

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
    // order. Each layer contributes both its `lua/` (libs reachable
    // via `require "ttymap.fmt"` etc.) and its `plugin/` (so a
    // directory plugin like `plugin/satellite/init.lua` can
    // `require "satellite.satellites"`).
    for layer in runtime_path() {
        prepend_package_path(&lua, &layer.join("lua"));
        prepend_package_path(&lua, &layer.join("plugin"));
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
// nvim-style two-tier layout per runtime layer:
//
// - `<layer>/plugin/*.lua` — auto-discovered plugins. Each script
//   self-registers via `ttymap.register_plugin / register_palette`
//   at top level.
// - `<layer>/lua/<name>.lua` — `require`-able lib scripts. NOT
//   auto-discovered. Plugins reach them via `require "<name>"`.
//
// Adding a new builtin = drop a `.lua` file under `runtime/plugin/`
// and `make install`. There is no Rust array to keep in sync.
//
// The runtime path itself is discovered at startup via
// [`runtimepath::resolve_runtime_path`]; see that module for the
// resolution order.

/// Register every bundled Lua plugin with the registrar by walking
/// `<layer>/plugin/*.lua` for each layer in `runtime_path`, in
/// priority order. Stem dedup means a higher-priority layer's plugin
/// shadows a lower-priority one with the same file name — drop a
/// `~/.config/ttymap/plugin/wiki.lua` to replace bundled `wiki`.
///
/// `disable` is the user-supplied opt-out list
/// (`ttymap.opt.plugins.disable`). A plugin whose stem matches any
/// entry is skipped at registration time. Each script's own
/// metadata drives how a registered plugin is wired — see
/// [`register_one`] for the kind dispatch.
pub fn register_builtin_plugins(
    runtime_path: &[PathBuf],
    disable: &[String],
    shared: Arc<ttymap::LuaHostShared>,
    r: &mut Registrar,
) {
    if runtime_path.is_empty() {
        log::warn!("lua: empty runtime path, no bundled plugins will load");
        return;
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for layer in runtime_path {
        let plugin_dir = layer.join("plugin");
        if !plugin_dir.is_dir() {
            continue;
        }
        register_plugins_in(&plugin_dir, Some(&mut seen), disable, shared.clone(), r);
    }
}

/// Whether the script asked to be skipped via `enabled = false`.
fn module_enabled(module: &Table) -> bool {
    !matches!(
        module.get::<mlua::Value>("enabled"),
        Ok(mlua::Value::Boolean(false))
    )
}

/// Read `module.footer_hints` as owned `(key, label)` pairs. Used as
/// a fallback when the script didn't call `register_footer_hint`
/// explicitly. Two accepted shapes per pair:
/// - `{ "Enter", "open" }` — positional 1-based array.
/// - `{ key = "Enter", label = "open" }` — named.
pub(crate) fn parse_footer_hints(module: &Table) -> Vec<(String, String)> {
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
    shared: Arc<ttymap::LuaHostShared>,
    r: &mut Registrar,
) {
    // Run the script once to capture its registration call. The
    // captured variant tells us which kind of plugin it is — Lua
    // is the source of truth, not file path or table layout.
    //
    // The `lua` returned here is the **setup state**: it holds the
    // module-level Lua locals from `register_*` setup, plus the
    // RegistryKey'd palette / keybind callbacks. We keep clones of
    // it in every closure that fires later so module-level vars
    // (e.g. an `enabled` flag) survive across the program's
    // lifetime — that's the hook for plugin-side toggle state.
    //
    // `plugin_ctl` is the open/close primitive surfaced as
    // `ttymap.plugin:open()` / `:close()` on this state only. The
    // factory closures below drain its atomics after each callback
    // (`open_request` → push a fresh component, `close_request` →
    // poll-tick close on the InstanceGuard wrapper). Per-instance
    // Lua states (built inside the factories) don't get this
    // userdata — they manage themselves via `ttymap.window:close()`.
    let shared_for_meta = shared.clone();
    let plugin_ctl = ttymap::PluginCtl::default();
    let (lua, captured, handles) = match bridge::handle::fresh_load(
        source,
        name,
        "lua-meta",
        shared_for_meta,
        Some(plugin_ctl.clone()),
    ) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
            return;
        }
    };

    // `kind == None` is the **pure-action plugin** shape: the script
    // declared one or more activation surfaces (palette command /
    // keybind / footer hint) but no `register_plugin / _palette`,
    // so there's no Component to push. `fresh_load` already rejected
    // the truly-empty case (no kind AND no surfaces); reaching here
    // with `None` means surfaces exist. Their `invoke` / callback
    // closures fire fire-and-forget host APIs (`ttymap.api.frame.export`,
    // `ttymap.map:jump`, …) that flow through `LuaHostHandles`.
    let is_pure_action_kind = captured.kind.is_none();
    let is_palette_kind = matches!(captured.kind, Some(ttymap::CapturedKind::Palette(_)));

    // Take ownership of the spec table when one exists. Pure-action
    // plugins skip this block — there's no module to read `enabled` /
    // `loop` / `footer_hints` from; only `captured.footer_hints` (the
    // explicit `register_footer_hint` calls) applies.
    let module_opt: Option<Table> = captured.kind.map(|k| match k {
        ttymap::CapturedKind::Plugin(t) | ttymap::CapturedKind::Palette(t) => t,
    });
    if let Some(module) = module_opt.as_ref()
        && !module_enabled(module)
    {
        log::info!("lua[{}]: enabled = false, skipping", name);
        drop(lua);
        return;
    }

    // Capture the plugin's per-frame `loop` callback if it declared
    // one. Optional and additive: scripts that don't use this field
    // continue to work through their existing `paint_on_map` / `poll`
    // hooks. The setup-state Lua is cloned (cheap Arc bump) into the
    // registry entry so the closure stays callable for the program's
    // lifetime. Pure-action plugins have no module, so no `loop`.
    if let Some(module) = module_opt.as_ref()
        && let Ok(loop_fn) = module.get::<mlua::Function>("loop")
    {
        match lua.create_registry_value(loop_fn) {
            Ok(loop_key) => {
                r.plugin_loops.register(crate::lua::registry::PluginLoop {
                    name,
                    lua: lua.clone(),
                    loop_fn: loop_key,
                });
            }
            Err(e) => {
                log::warn!("lua[{}]: failed to register loop fn: {}", name, e);
            }
        }
    }

    // Read footer_hints from the module before it's consumed by
    // the kind-specific branch below. Spec-level `footer_hints`
    // is the legacy fallback; explicit
    // `ttymap.register_footer_hint(...)` calls win when present.
    let footer_hints = if !captured.footer_hints.is_empty() {
        captured.footer_hints.clone()
    } else if let Some(module) = module_opt.as_ref() {
        parse_footer_hints(module)
    } else {
        Vec::new()
    };

    // Up-front validate componented plugins so a syntax error
    // surfaces as one log line instead of a noisy first-
    // activation failure later. The validation state is
    // throwaway; activation rebuilds a fresh state per push.
    // Pure-action plugins have nothing to pre-build (no Component
    // is ever pushed), so skip validation — `fresh_load` already
    // confirmed the script parses and registered surfaces.
    let valid = if is_pure_action_kind {
        Ok(())
    } else if is_palette_kind {
        LuaPaletteProvider::from_source(source, name, shared.clone()).map(|_| ())
    } else {
        LuaComponent::from_source(source, name, shared.clone()).map(|_| ())
    };
    if let Err(e) = valid {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }
    // Hand the setup state's `LuaHostHandles` over to the
    // registrar so the App can drain its receivers per frame.
    // Setup-state callbacks (palette command invoke, register_keybind
    // callback, plugin-level `loop`, and any `ttymap.api.window.open`
    // / `palette.open` spec callbacks) flip these senders. Without
    // this push the receivers would just sit (latent bug pre-A7).
    //
    // Pure-action plugins also need this — their palette `invoke`
    // callbacks fire fire-and-forget APIs (`ttymap.map:jump`,
    // `ttymap.api.frame.export`) that flow through these very
    // channels.
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
        push_plugin_entry(&shared, name, &key_hint, &label, &footer_hints);
    }

    // Explicit-callback paths: each register_palette_command and
    // register_keybind from the script gets its own factory. The
    // factory runs the captured Lua callback in the persistent
    // setup state and then drains the per-plugin open/close
    // primitives:
    //
    // - The callback ran some plugin-side state mutation
    //   (`enabled = not enabled`, append to a list, …).
    // - If it called `ttymap.plugin:open()` while running,
    //   `open_request` is now true — the factory builds a fresh
    //   instance and pushes it onto the stack.
    // - If it called `ttymap.plugin:close()`, the running
    //   instance's [`InstanceGuard`] picks up `close_request` on
    //   its next `Component::poll` tick and calls `win.close()`.
    //
    // Whether a plugin allows multiple instances on the stack is
    // a Lua-side decision now — Rust no longer enforces single-
    // instance. A plugin that wants no-double-window keeps an
    // `enabled` flag (or stash the Window handle from `:open()`)
    // and only calls `plugin:open()` when the flag flips on.
    use crate::compositor::{Activation, Component, PaletteEntry};
    use crossterm::event::{KeyCode, KeyModifiers};
    let build_factory = |gate_key: mlua::RegistryKey,
                         lua_clone: mlua::Lua,
                         shared_clone: Arc<ttymap::LuaHostShared>,
                         ctl: ttymap::PluginCtl|
     -> crate::compositor::SpawnComponent {
        if is_pure_action_kind {
            // Pure-action plugins have no Component to build at all;
            // their `invoke` fires fire-and-forget host APIs. Drain
            // both atomics so a stray `ttymap.plugin:open()/close()`
            // from one of these callbacks is swallowed cleanly.
            Box::new(move |_ctx| {
                run_lua_callback(&lua_clone, &gate_key, name);
                ctl.open_request
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                ctl.close_request
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                None
            })
        } else if is_palette_kind {
            Box::new(move |_ctx| {
                run_lua_callback(&lua_clone, &gate_key, name);
                if !ctl
                    .open_request
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    return None;
                }
                let provider = LuaPaletteProvider::from_source(source, name, shared_clone.clone())
                    .unwrap_or_else(|e| {
                        log::warn!("lua[{}]: re-load failed: {}", name, e);
                        LuaPaletteProvider::from_source(
                            "ttymap.register_palette({})",
                            "lua-fallback",
                            shared_clone.clone(),
                        )
                        .expect("trivial provider always loads")
                    });
                let palette = crate::palette::PaletteComponent::with_provider(provider);
                Some(Box::new(InstanceGuard {
                    inner: palette,
                    close_request: ctl.close_request.clone(),
                }) as Box<dyn Component>)
            })
        } else {
            Box::new(move |_ctx| {
                run_lua_callback(&lua_clone, &gate_key, name);
                if !ctl
                    .open_request
                    .swap(false, std::sync::atomic::Ordering::Relaxed)
                {
                    return None;
                }
                let inner = component_or_placeholder(name, source, shared_clone.clone());
                Some(Box::new(InstanceGuard {
                    inner,
                    close_request: ctl.close_request.clone(),
                }) as Box<dyn Component>)
            })
        }
    };

    for cmd in captured.palette_commands {
        let factory = build_factory(cmd.invoke, lua.clone(), shared.clone(), plugin_ctl.clone());
        r.palette_entries.push(PaletteEntry {
            label: cmd.label,
            hint: cmd.hint,
            name,
            spawn: factory,
        });
    }
    for bind in captured.keybinds {
        let factory = build_factory(
            bind.callback,
            lua.clone(),
            shared.clone(),
            plugin_ctl.clone(),
        );
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

/// Wrapper that forwards every [`Component`] call to `inner` and
/// also drains a shared `close_request` flag on each `poll` tick:
/// when the setup-state Lua side calls `ttymap.plugin:close()`,
/// the next `poll` consumes the flag and calls `win.close()`.
///
/// This is the only Rust-side state in the plugin lifecycle. The
/// component itself doesn't know about it; the wrapper is what
/// translates the plugin author's Lua-side `:close()` request
/// into a stack pop, regardless of which component currently
/// has focus.
struct InstanceGuard<C: crate::compositor::Component> {
    inner: C,
    close_request: Arc<std::sync::atomic::AtomicBool>,
}

impl<C: crate::compositor::Component> crate::compositor::Component for InstanceGuard<C> {
    fn handle_event(
        &mut self,
        event: crossterm::event::KeyEvent,
        win: &mut crate::compositor::window::Window,
    ) {
        self.inner.handle_event(event, win)
    }
    fn render(&self, win: &mut crate::compositor::window::RenderWindow) {
        self.inner.render(win)
    }
    fn paint_on_map(&self, p: &mut crate::compositor::MapApi<'_>) {
        self.inner.paint_on_map(p)
    }
    fn poll(&mut self, win: &mut crate::compositor::window::Window) {
        self.inner.poll(win);
        if self
            .close_request
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            win.close();
        }
    }
    fn poll_overlay(&mut self, win: &mut crate::compositor::window::OverlayWindow) {
        self.inner.poll_overlay(win)
    }
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.inner.footer_hints()
    }
    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

/// Run a captured Lua callback (palette command's invoke or
/// keybind's callback). The callback's return value is ignored —
/// it signals open/close intent via `ttymap.plugin:open()` /
/// `:close()` instead, and the caller drains those atomics after
/// this returns. Errors are logged with the plugin's name but
/// don't propagate.
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
fn push_plugin_entry(
    shared: &Arc<ttymap::LuaHostShared>,
    name: &str,
    key: &str,
    label: &str,
    footer_hints: &[(String, String)],
) {
    shared.push_palette_entry(ttymap::PluginEntry {
        name: name.to_string(),
        key: key.to_string(),
        label: label.to_string(),
        footer_hints: footer_hints.to_vec(),
    });
}

/// Try to build a `LuaComponent` from `source`; on failure log + fall
/// back to a no-op module so the host stays alive.
fn component_or_placeholder(
    name: &'static str,
    source: &'static str,
    shared: Arc<ttymap::LuaHostShared>,
) -> LuaComponent {
    LuaComponent::from_source(source, name, shared.clone()).unwrap_or_else(|e| {
        log::warn!("lua[{}]: re-load failed: {}", name, e);
        LuaComponent::from_source("ttymap.register_plugin({})", name, shared)
            .expect("trivial Lua module always loads")
    })
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
    disable: &[String],
    shared: Arc<ttymap::LuaHostShared>,
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
        if disable.iter().any(|d| d == &stem) {
            log::info!(
                "lua[{}]: disabled via ttymap.opt.plugins.disable, skipping",
                stem
            );
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
        register_one(name, source, shared.clone(), r);
    }
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
        let shared = ttymap::LuaHostShared::empty();
        let mut r = Registrar::default();
        let rtp = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime")];
        register_builtin_plugins(&rtp, &[], shared, &mut r);

        let palette: std::collections::HashSet<String> = r
            .palette_entries
            .iter()
            .map(|e| e.label.to_lowercase())
            .collect();
        // `r.plugin_loops` doesn't carry a name; sanity-check the
        // total count of registered plugin loops as a lower bound.
        // info / scalebar / attribution / center each register a
        // `register_plugin` + `loop` callback (after Phase B); Phase B
        // panel migrations (aircraft / quake / wiki / …) also register
        // a `loop` for their panel, so this is a lower bound rather
        // than equality.
        let always_on_count = r.plugin_loops.len();

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
        assert!(
            always_on_count >= 4,
            "info/scalebar/attribution/center should each register a plugin loop (got {always_on_count})"
        );
    }

    /// Two `LuaComponent` instances coexist on the compositor stack
    /// — there's no Rust-side identity dedup, so distinct script
    /// load paths produce independent components, even with the
    /// same internal `name`. nvim-style: stack push is unconditional.
    #[test]
    fn two_lua_components_coexist_on_compositor_stack() {
        use crate::compositor::Component;
        let shared = ttymap::LuaHostShared::empty();
        let iss = LuaComponent::from_source(
            r#"ttymap.register_plugin({ name = "iss", render = function() return {} end })"#,
            "iss",
            shared.clone(),
        )
        .expect("build iss");
        let hubble = LuaComponent::from_source(
            r#"ttymap.register_plugin({ name = "hubble", render = function() return {} end })"#,
            "hubble",
            shared,
        )
        .expect("build hubble");
        // Display names come from the script's `name` field — the
        // user-facing label, not a registration identity.
        assert_eq!(iss.name(), "iss");
        assert_eq!(hubble.name(), "hubble");
    }

    /// Helper for spec-table inspection tests. Runs the source in a
    /// throwaway Lua state, returns the captured spec table along
    /// with its variant tag so the test can assert on both. Mirrors
    /// what `register_one` does at registration time.
    fn parse_spec(source: &str, name: &str) -> (mlua::Lua, ttymap::CapturedRegistration) {
        let shared = ttymap::LuaHostShared::empty();
        let (lua, captured, _handles) =
            bridge::handle::fresh_load(source, name, "lua-test", shared, None).expect("load");
        (lua, captured)
    }

    fn captured_table(c: &ttymap::CapturedRegistration) -> &Table {
        match c.kind.as_ref().expect("script registered something") {
            ttymap::CapturedKind::Plugin(t) | ttymap::CapturedKind::Palette(t) => t,
        }
    }

    #[test]
    fn register_plugin_yields_plugin_variant() {
        let (_lua, c) = parse_spec(r#"ttymap.register_plugin({ name = "x" })"#, "x");
        assert!(matches!(c.kind, Some(ttymap::CapturedKind::Plugin(_))));
        let t = captured_table(&c);
        assert!(module_enabled(t));
        // No surfaces declared: empty palette_commands + keybinds.
        assert!(c.palette_commands.is_empty());
        assert!(c.keybinds.is_empty());
    }

    #[test]
    fn register_palette_yields_palette_variant() {
        let (_lua, c) = parse_spec(r#"ttymap.register_palette({ name = "x" })"#, "x");
        assert!(matches!(c.kind, Some(ttymap::CapturedKind::Palette(_))));
    }

    #[test]
    fn enabled_false_surfaces_through_module_enabled() {
        let (_lua, c) = parse_spec(
            r#"ttymap.register_plugin({ name = "x", enabled = false })"#,
            "x",
        );
        let t = captured_table(&c);
        assert!(!module_enabled(t));
    }

    #[test]
    fn explicit_palette_command_and_keybind_are_captured() {
        let (_lua, c) = parse_spec(
            r#"
            ttymap.register_plugin({ name = "x" })
            ttymap.register_palette_command({ label = "Toggle x", invoke = function() return true end })
            ttymap.register_keybind("i", function() return true end)
            "#,
            "x",
        );
        assert_eq!(c.palette_commands.len(), 1);
        assert_eq!(c.palette_commands[0].label, "Toggle x");
        assert_eq!(c.keybinds.len(), 1);
        assert_eq!(c.keybinds[0].key, 'i');
    }

    #[test]
    fn parse_footer_hints_accepts_both_shapes() {
        let (_lua, c) = parse_spec(
            r#"ttymap.register_plugin({
                name = "x",
                footer_hints = {
                    { "Enter", "open" },
                    { key = "Esc", label = "back" },
                },
            })"#,
            "x",
        );
        let hints = parse_footer_hints(captured_table(&c));
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0], ("Enter".into(), "open".into()));
        assert_eq!(hints[1], ("Esc".into(), "back".into()));
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
            ttymap.register_plugin({
                name = "demo",
                render = function() return {} end,
                footer_hints = {
                    { "Enter", "open" },
                    { "Esc",   "close" },
                },
            })
            ttymap.register_palette_command({ label = "Toggle demo", invoke = function() return true end })
            ttymap.register_keybind("d", function() return true end)
            "#,
        );

        let shared = ttymap::LuaHostShared::empty();
        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], shared.clone(), &mut r);

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
            r#"ttymap.register_plugin({ name = "first", render = function() return {} end })
            ttymap.register_palette_command({ label = "first", invoke = function() return true end })"#,
        );
        write_plugin(
            &dir,
            "second.lua",
            r#"ttymap.register_plugin({ name = "second", render = function() return {} end })
            ttymap.register_palette_command({ label = "second", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
        let ls = labels(&r);
        assert!(ls.iter().any(|l| l.contains("first")), "got {:?}", ls);
        assert!(ls.iter().any(|l| l.contains("second")), "got {:?}", ls);
    }

    #[test]
    fn dir_discovery_skips_non_lua_files() {
        let dir = temp_plugins_dir("skip-non-lua");
        write_plugin(
            &dir,
            "ok.lua",
            r#"ttymap.register_plugin({ name = "ok" })
            ttymap.register_palette_command({ label = "ok", invoke = function() return true end })"#,
        );
        // README, backup files, etc. should be ignored.
        std::fs::write(dir.join("README.md"), "ignore me").unwrap();
        std::fs::write(dir.join("ok.lua.bak"), "ignore me too").unwrap();

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
        let ls = labels(&r);
        assert_eq!(ls.len(), 1, "got {:?}", ls);
        assert!(ls[0].contains("ok"));
    }

    #[test]
    fn dir_discovery_honours_module_enabled_false() {
        let dir = temp_plugins_dir("self-disable");
        write_plugin(
            &dir,
            "alpha.lua",
            r#"ttymap.register_plugin({ name = "alpha" })
            ttymap.register_palette_command({ label = "alpha", invoke = function() return true end })"#,
        );
        // beta opts itself out — the file stays, but the plugin
        // doesn't register.
        write_plugin(
            &dir,
            "beta.lua",
            r#"ttymap.register_plugin({ name = "beta", enabled = false })
            ttymap.register_palette_command({ label = "beta", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
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
            r#"ttymap.register_plugin({ name = "biggie", render = function() return {} end })
            ttymap.register_palette_command({ label = "biggie", invoke = function() return true end })"#,
        )
        .expect("write init.lua");

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
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
            r#"ttymap.register_plugin({ format = function(s) return s end })"#,
        )
        .expect("write fmt.lua");

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
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
            r#"ttymap.register_plugin({ name = "explicit", enabled = true })
            ttymap.register_palette_command({ label = "explicit", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
        assert!(labels(&r).iter().any(|l| l.contains("explicit")));
    }

    #[test]
    fn dir_discovery_skips_broken_lua_but_keeps_going() {
        let dir = temp_plugins_dir("broken");
        write_plugin(&dir, "broken.lua", "this is not lua syntax !!!");
        write_plugin(
            &dir,
            "ok.lua",
            r#"ttymap.register_plugin({ name = "ok", render = function() return {} end })
            ttymap.register_palette_command({ label = "ok", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
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
        register_plugins_in(&dir, None, &[], ttymap::LuaHostShared::empty(), &mut r);
        assert!(r.palette_entries.is_empty());
    }
}
