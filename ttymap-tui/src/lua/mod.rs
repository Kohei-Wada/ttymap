//! Lua runtime scaffold for scripted plugins.
//!
//! Owns the shared [`mlua::Lua`] state. The bridge surface (Component
//! adapter, MapApi, widget descriptors, etc.) lives in the submodules
//! below — see `docs/lua-architecture.md` for the full per-namespace
//! tour.
//!
//! Defaults that are deliberate and not provisional:
//! - **Lua 5.4** via the `vendored` mlua feature: portable, no
//!   system Lua dep, ~1 MB binary growth.
//! - **No sandbox**: ttymap is single-user; trust the plugin author
//!   (the maintainer themself for now).
//! - **Errors are logged, not propagated**: a buggy plugin must not
//!   crash the host. Helpers in this module wrap mlua results with
//!   `log::warn!` + recovery default.

pub mod api;
pub mod bridge;
pub mod handle;
pub mod host;
pub mod init_lua;
pub mod map_api;
pub mod registrar;
pub mod runtimepath;
pub mod tick;
pub mod vm;

pub use bridge::palette_provider::LuaPaletteProvider;
pub use handle::LuaHandle;
pub use host::LuaHostShared;
pub use init_lua::read_init_lua_config_only;
pub use map_api::MapApi;
pub use registrar::{LuaRegistry, LuaRegistryHandle, new_lua_registry};
pub use runtimepath::{resolve_runtime_path, runtime_path, set_runtime_path};
pub use vm::new_lua;

use std::sync::Arc;

use crate::UserCommand;
use crate::compositor::op;
use crate::config::Config;
use crate::input::KeyMap;
use crate::input::keymap::KeybindingOverrides;

/// Result of [`build_subsystem`].
///
/// The runtime-held [`LuaHandle`] is **already constructed** by
/// `build_subsystem` itself — the App just stores it. The
/// `registry` handle is shared with `BaseLayer` and the palette
/// installer so plugin `:remove()` can mutate it at runtime.
pub struct LuaSubsystem {
    /// Runtime handle to the Lua subsystem — semantic surface
    /// App uses to observe state changes and tick plugins.
    pub handle: LuaHandle,
    /// Lua-agnostic pub/sub primitive. Shared with every
    /// `EventHandle` userdata returned to Lua (for `:remove()`) and
    /// with [`crate::app::App`], which drains its accumulated event
    /// buffer once per loop iteration onto this bus (#334, #336).
    pub bus: std::rc::Rc<crate::event::EventBus>,
    /// Live registry of Lua-registered activations + palette
    /// entries. Cloned into `BaseLayer` (for keypress dispatch) and
    /// the palette installer (for the `:` activation's per-open
    /// `CommandSeed` snapshot). Lua-side `PaletteCommandHandle:remove()`
    /// / `KeybindHandle:remove()` mutably borrow it to drop entries.
    pub registry: LuaRegistryHandle,
    /// `[<key> <label>]` footer hints harvested from the registry's
    /// palette entries at startup. Static for the program lifetime —
    /// adding / removing entries at runtime does not refresh this
    /// list (footer redraw avoidance trade-off; trivial to revisit
    /// if dynamic registration becomes a use case).
    pub footer_hints: Vec<(&'static str, &'static str)>,
}

/// Build the Lua plugin subsystem: create the shared VM, install
/// the API surface, run the init.lua chain (which `require`s every
/// bundled plugin as `plugin.<name>` via standard `package.path`,
/// and lets the user's `~/.config/ttymap/init.lua` add or override),
/// then return the populated subsystem alongside the parsed
/// [`Config`] and [`KeybindingOverrides`].
///
/// nvim-style: a single Lua state hosts init.lua, every bundled
/// plugin, and every user plugin. The whole bootstrap is one
/// function — there is no separate plugin-discovery walker.
///
/// Bootstrap order:
///
/// 1. Build registry / shared / ops / bus.
/// 2. `vm::new_lua()` — fresh VM with the lib searcher + extended
///    `package.path` for runtime layers (covers both libs and
///    `plugin.<name>` modules under `<layer>/lua/`).
/// 3. Install the `ttymap` global's pre-pass surface (`opt`,
///    `keymap`) so init.lua can mutate config defaults.
/// 4. `api::install` — extends `ttymap` with the runtime API
///    (`register_*`, `on_event`, `http`, `map`, …). Plugins need
///    these *during* their require, so this MUST run before init.lua.
/// 5. Run the init.lua chain (system → user). All `require
///    "plugin.<name>"` calls fire here; each `register_*` call
///    inside a plugin chunk pushes directly into the live registry
///    / bus.
/// 6. `read_back` parses `ttymap.opt.*` into `Config`; clone
///    `KeybindingOverrides` from the keymap-state cell.
/// 7. Build the live `KeyMap` from overrides, fold its rows into
///    `shared.keymap_entries` so `ttymap.help:keymap_entries()` sees
///    them at render time. The brief register-time emptiness is
///    invisible — help.lua queries lazily.
/// 8. Harvest footer hints, assemble [`LuaHandle`].
///
/// **Does not install the palette** — that step runs from the
/// composition root after this returns, draining `palette_entries`
/// into a default `:`-palette provider.
pub fn build_subsystem(defaults: Config) -> (LuaSubsystem, Config, KeybindingOverrides, KeyMap) {
    let registry = new_lua_registry();
    let shared = Arc::new(LuaHostShared::new());
    let bus = std::rc::Rc::new(crate::event::EventBus::default());
    let ops = op::new_ops_buffer();

    let lua = vm::new_lua();

    // init.lua's `ttymap` pre-pass (`opt` + `keymap`) — must run
    // before `api::install` so `api::install` extends the existing
    // table rather than clobbering it.
    let keymap_state = match init_lua::install_ttymap_global(&lua, &defaults) {
        Ok(state) => state,
        Err(e) => {
            log::warn!("lua: install_ttymap_global failed: {} — using defaults", e);
            let keymap = KeyMap::with_overrides(&KeybindingOverrides::new());
            return (
                LuaSubsystem {
                    handle: LuaHandle::new(bus.clone(), Vec::new(), ops, shared),
                    bus,
                    registry,
                    footer_hints: Vec::new(),
                },
                defaults,
                KeybindingOverrides::new(),
                keymap,
            );
        }
    };

    // API surface. Every plugin sees `ttymap.register_*` /
    // `ttymap.on_event` / `ttymap.http` / `ttymap.map` / … the
    // moment its require fires.
    let host_handles = match api::install(
        &lua,
        shared.clone(),
        ops.clone(),
        bus.clone(),
        registry.clone(),
    ) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("lua: api::install failed: {} — plugins disabled", e);
            let keymap = KeyMap::with_overrides(&KeybindingOverrides::new());
            return (
                LuaSubsystem {
                    handle: LuaHandle::new(bus.clone(), Vec::new(), ops, shared),
                    bus,
                    registry,
                    footer_hints: Vec::new(),
                },
                defaults,
                KeybindingOverrides::new(),
                keymap,
            );
        }
    };

    // Run the bundled init.lua. It `require`s every bundled
    // plugin as `plugin.<name>` (resolves via the prepended
    // `package.path`), then pulls in user config via
    // `require("ttymap.user_config").load()`. The user-config
    // path lives entirely on the Lua side — Rust never names it.
    // Errors are logged + recovered.
    init_lua::run_system_init_lua(&lua);

    // Read `ttymap.opt.*` mutations back into `Config`. Type errors
    // silently fall back to the seeded value — a typo doesn't crash
    // the host.
    let config = init_lua::read_back(&lua, &defaults).unwrap_or_else(|e| {
        log::warn!("lua: read_back failed: {} — falling back to defaults", e);
        defaults
    });

    let keymap_overrides = keymap_state.borrow().clone();
    let keymap = KeyMap::with_overrides(&keymap_overrides);

    // Populate the keymap-entries snapshot help reads at render time.
    // Live data — runtime keymap overrides surface here.
    shared.set_keymap_entries(UserCommand::keymap_help_entries(&keymap));

    // Harvest BaseLayer's footer hints from the registry's palette
    // entries. Snapshot at startup — we don't refresh on runtime
    // remove. The footer slot is `[<key> <label>]`, built directly
    // from each entry's keybinding and label. No keybinding ⇒ no
    // footer slot.
    let footer_hints: Vec<(&'static str, &'static str)> = registry
        .borrow()
        .palette_entries()
        .iter()
        .filter(|(_, e)| !e.hint.is_empty())
        .map(|(_, e)| {
            let key: &'static str = Box::leak(e.hint.clone().into_boxed_str());
            let label: &'static str = Box::leak(e.label.clone().into_boxed_str());
            (key, label)
        })
        .collect();

    let handle = LuaHandle::new(bus.clone(), vec![host_handles], ops, shared);

    (
        LuaSubsystem {
            handle,
            bus,
            registry,
            footer_hints,
        },
        config,
        keymap_overrides,
        keymap,
    )
}

#[cfg(test)]
mod tests {
    use super::vm::{install_builtin_searcher, prepend_package_path};
    use super::*;
    use mlua::{Lua, Result};
    use std::path::PathBuf;

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

    /// Helper for inspection tests. Drives the host API install (so
    /// `ttymap.register_*` etc. exist), runs the source as a plain
    /// Lua chunk, and returns the live `(lua, registry, bus)` tuple
    /// so callers can read what landed in the registry / bus
    /// directly. No deferred capture — registrations apply
    /// immediately as the chunk runs.
    fn run_in_fresh_vm(
        source: &'static str,
    ) -> (
        mlua::Lua,
        LuaRegistryHandle,
        std::rc::Rc<crate::event::EventBus>,
    ) {
        let lua = new_lua();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let registry = new_lua_registry();
        api::install(
            &lua,
            host::LuaHostShared::empty(),
            op::new_ops_buffer(),
            bus.clone(),
            registry.clone(),
        )
        .expect("install ttymap");
        lua.load(source).exec().expect("exec source");
        (lua, registry, bus)
    }

    /// Build a Lua VM with `package.path` extended for a custom
    /// runtime layer. Returns `(lua, registry, shared, bus)` so tests
    /// can drive `lua.load(r#"require 'plugin.<name>'"#).exec()` and
    /// assert what landed in the registry / shared snapshot.
    ///
    /// Plugins resolve as standard Lua modules under
    /// `<layer>/lua/plugin/<name>.lua` via the prepended
    /// `package.path` — there is no custom searcher, no host-side
    /// plugin attribution.
    fn fresh_lua_layer_state(
        layer: &Path,
    ) -> (
        mlua::Lua,
        LuaRegistryHandle,
        Arc<host::LuaHostShared>,
        std::rc::Rc<crate::event::EventBus>,
    ) {
        runtimepath::ensure_runtime_path_for_tests();
        let lua = new_lua();
        // Prepend the test layer's `lua/` to package.path so
        // `require "plugin.<name>"` (and lib requires) resolve from
        // the temp dir before the global runtime layers.
        vm::prepend_package_path(&lua, &layer.join("lua"));
        let shared = host::LuaHostShared::empty();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let registry = new_lua_registry();
        api::install(
            &lua,
            shared.clone(),
            op::new_ops_buffer(),
            bus.clone(),
            registry.clone(),
        )
        .expect("install ttymap");

        (lua, registry, shared, bus)
    }

    /// Build a private temp directory rooted at the OS's temp dir.
    /// `unique` should differ per test so parallel runs don't
    /// stomp on each other.
    fn temp_layer(unique: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ttymap-lua-test-{}", unique));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lua").join("plugin")).expect("mkdir lua/plugin");
        dir
    }

    fn write_plugin(layer: &Path, file_name: &str, lua: &str) {
        std::fs::write(layer.join("lua").join("plugin").join(file_name), lua)
            .expect("write plugin");
    }

    fn write_plugin_dir(layer: &Path, name: &str, lua: &str) {
        let dir = layer.join("lua").join("plugin").join(name);
        std::fs::create_dir_all(&dir).expect("mkdir plugin dir");
        std::fs::write(dir.join("init.lua"), lua).expect("write plugin init.lua");
    }

    fn labels(r: &LuaRegistryHandle) -> Vec<String> {
        r.borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect()
    }

    #[test]
    fn register_palette_command_and_keybind_land_in_registry() {
        // `ttymap.register_palette_command(spec)` + `ttymap.register_keybind(key, fn)`
        // push directly into the live `LuaRegistry`. No deferred
        // capture — the entries are visible the moment the chunk
        // returns.
        let (_lua, registry, _bus) = run_in_fresh_vm(
            r#"
            ttymap.register_palette_command({ label = "Toggle x", invoke = function() return true end })
            ttymap.register_keybind("i", function() return true end)
            "#,
        );
        let r = registry.borrow();
        assert_eq!(r.palette_entry_count(), 1);
        let labels: Vec<String> = r
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect();
        assert_eq!(labels, vec!["Toggle x"]);
        assert_eq!(r.activation_count(), 1);
        assert!(
            r.find_activation(
                crossterm::event::KeyCode::Char('i'),
                crossterm::event::KeyModifiers::NONE
            )
            .is_some()
        );
    }

    #[test]
    fn on_tick_subscriptions_land_on_the_bus() {
        // Each `ttymap.api.frame.on_tick(fn)` call subscribes
        // directly against the bus.
        let (_lua, _registry, bus) = run_in_fresh_vm(
            r#"
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.api.frame.on_tick(function(_map) end)
            "#,
        );
        assert_eq!(bus.count("tick"), 2);
    }

    #[test]
    fn require_pushes_palette_entry_into_registry() {
        // `require "plugin.demo"` from init.lua-equivalent driver
        // executes the chunk via standard `package.path`; the
        // chunk's `register_*` calls push directly into the live
        // registry.
        let layer = temp_layer("require-pushes-entry");
        write_plugin(
            &layer,
            "demo.lua",
            r#"
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.register_palette_command({ label = "Toggle demo", hint = "d", invoke = function() return true end })
            ttymap.register_keybind("d", function() return true end)
            "#,
        );

        let (lua, registry, shared, _bus) = fresh_lua_layer_state(&layer);
        lua.load(r#"require "plugin.demo""#)
            .exec()
            .expect("require plugin.demo");

        let labels: Vec<String> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect();
        assert!(
            labels.iter().any(|l| l == "Toggle demo"),
            "plugin's palette entry should be pushed into the registry, got {:?}",
            labels
        );

        // Help snapshot: the palette-command's `hint` populates a help row.
        let entries = shared.help_entries.lock().expect("lock help_entries");
        let demo = entries
            .iter()
            .find(|e| e.label == "Toggle demo")
            .expect("help row should exist for `Toggle demo`");
        assert_eq!(demo.key, "d");
    }

    #[test]
    fn directory_plugin_is_required_via_init_lua() {
        // `<layer>/lua/plugin/wiki/init.lua` is the plugin entry;
        // user does `require "plugin.wiki"`. Mirrors a directory-
        // shaped bundled plugin (e.g. travel/, satellite/).
        let layer = temp_layer("require-dir-plugin");
        write_plugin_dir(
            &layer,
            "biggie",
            r#"ttymap.register_palette_command({ label = "biggie", invoke = function() return true end })"#,
        );

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);
        lua.load(r#"require "plugin.biggie""#)
            .exec()
            .expect("require plugin.biggie");
        let ls = labels(&registry);
        assert!(
            ls.iter().any(|l| l.contains("biggie")),
            "expected biggie/init.lua to register, got {:?}",
            ls
        );
    }

    #[test]
    fn lib_require_does_not_register_anything() {
        // A lib at `lua/ttymap/foo.lua` is required via
        // `require "ttymap.foo"`. Resolves through package.path the
        // same way a plugin require does — there is no special
        // attribution path. The lib just returns a value; no
        // `register_*` calls fire, so the registry stays empty.
        let layer = temp_layer("lib-require");
        std::fs::create_dir_all(layer.join("lua").join("ttymap")).expect("mkdir lua/ttymap");
        std::fs::write(
            layer.join("lua").join("ttymap").join("foo.lua"),
            r#"return { value = 42 }"#,
        )
        .expect("write lib");

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);
        let v: i64 = lua
            .load(r#"return require("ttymap.foo").value"#)
            .eval()
            .expect("require ttymap.foo");
        assert_eq!(v, 42);
        assert_eq!(registry.borrow().palette_entry_count(), 0);
    }

    #[test]
    fn requiring_same_plugin_twice_only_fires_once() {
        // Lua's `package.loaded` cache: after the first require runs
        // the chunk, subsequent `require "plugin.demo"` calls return
        // the cached value without re-executing. So system + user
        // init.lua both doing the same require register exactly
        // one entry.
        let layer = temp_layer("dedup");
        write_plugin(
            &layer,
            "demo.lua",
            r#"ttymap.register_palette_command({ label = "demo", invoke = function() return true end })"#,
        );

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);
        lua.load(r#"require "plugin.demo"; require "plugin.demo""#)
            .exec()
            .expect("double require");

        assert_eq!(
            registry.borrow().palette_entry_count(),
            1,
            "duplicate require must not double-register"
        );
    }

    #[test]
    fn submodule_require_resolves_as_plain_module() {
        // `require "plugin.travel.routes.italy"` from inside the
        // travel plugin's init.lua resolves through package.path
        // the same way the top-level `require "plugin.travel"`
        // does — both are just Lua module requires now. The
        // submodule returns a value used by the parent plugin's
        // `register_palette_command` call.
        let layer = temp_layer("submodule");
        let travel_dir = layer.join("lua").join("plugin").join("travel");
        std::fs::create_dir_all(travel_dir.join("routes")).expect("mkdir routes");
        std::fs::write(
            travel_dir.join("routes").join("italy.lua"),
            r#"return { city = "Rome" }"#,
        )
        .expect("write submodule");
        std::fs::write(
            travel_dir.join("init.lua"),
            r#"
            local italy = require "plugin.travel.routes.italy"
            ttymap.register_palette_command({
                label = italy.city,
                invoke = function() return true end,
            })
            "#,
        )
        .expect("write travel/init.lua");

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);
        lua.load(r#"require "plugin.travel""#)
            .exec()
            .expect("require plugin.travel");
        let labels: Vec<String> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect();
        // Exactly one palette entry — the submodule returned a
        // value, parent registered with that value as label.
        assert_eq!(labels, vec!["Rome"]);
    }

    #[test]
    fn package_path_extension_lets_require_find_siblings() {
        // Drop a `helper.lua` into a tempdir, point Lua's
        // `package.path` at it, then `require "helper"` from a
        // plain Lua chunk. The require must resolve to the file
        // we wrote — proves the extension is wired.
        let dir = std::env::temp_dir().join("ttymap-lua-test-pkgpath-helper");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
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
        let dir = std::env::temp_dir().join("ttymap-lua-test-pkgpath-passthrough");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let lua = Lua::new();
        prepend_package_path(&lua, &dir);
        let res: mlua::Result<i64> = lua.load(r#"return require("doesnotexist")"#).eval();
        assert!(res.is_err(), "non-existent require should still error");
    }

    /// The whole point of unifying init.lua's VM with the plugin VM:
    /// a config-holder lib at `runtime/lua/ttymap/<name>.lua` is
    /// reachable via `require "ttymap.<name>"` from BOTH init.lua
    /// AND a plugin script, and both round-trips return the same
    /// cached table — so a user's init.lua can pre-mutate the table
    /// and the plugin sees the mutation when it loads later.
    #[test]
    fn init_lua_can_seed_config_for_a_plugin_via_require() {
        let layer = temp_layer("init-seeds-plugin-config");
        std::fs::create_dir_all(layer.join("lua").join("ttymap")).expect("mkdir lua/ttymap");
        std::fs::write(
            layer.join("lua").join("ttymap").join("myplug.lua"),
            "return { label = 'default-label' }",
        )
        .expect("write lib");
        write_plugin(
            &layer,
            "myplug.lua",
            r#"
            local cfg = require "ttymap.myplug"
            ttymap.register_palette_command({
                label = cfg.label,
                invoke = function() return true end,
            })
            "#,
        );

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);
        // init.lua-style pre-pass: mutate the lib's table BEFORE the
        // plugin requires it. Lua's module cache makes the plugin's
        // `require "ttymap.myplug"` return the same table.
        lua.load(r#"require("ttymap.myplug").label = "from-init""#)
            .exec()
            .expect("init pre-pass");
        // Now activate the plugin; it should pick up the mutated
        // label.
        lua.load(r#"require "plugin.myplug""#)
            .exec()
            .expect("require plugin.myplug");

        let labels: Vec<String> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect();
        assert!(
            labels.iter().any(|l| l == "from-init"),
            "plugin should read the mutated lib's `label`, got {:?}",
            labels,
        );
    }

    /// End-to-end: run the merged bootstrap against the in-repo
    /// `runtime/init.lua` (which `require`s every bundled plugin)
    /// and assert each plugin lands in the registry. Set-membership
    /// rather than counts so adding a builtin doesn't require
    /// updating magic numbers.
    #[test]
    fn every_bundled_script_registers_via_init_lua_chain() {
        runtimepath::ensure_runtime_path_for_tests();
        let (subsystem, _config, _km, _keymap) = build_subsystem(Config::default());

        let palette: std::collections::HashSet<String> = subsystem
            .registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.to_lowercase())
            .collect();

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
            "travel",
            "terminator",
            "geo quiz",
        ] {
            assert!(
                palette.iter().any(|l| l.contains(stem)),
                "expected `{stem}` palette entry, got {palette:?}",
            );
        }
    }

    /// Driver test for the Lua-driven user-config flow: bundled
    /// `runtime/init.lua` runs the bundled plugin set FIRST, then
    /// pulls in user config LAST via
    /// `require("ttymap.user_config").load()`. User init.lua
    /// activates user plugins via `require "plugin.<name>"` (same
    /// path as bundled — both go through `package.path`).
    ///
    /// Stubs `ttymap.user_config` via `package.preload` so the test
    /// doesn't have to mutate the real `~/.config/ttymap/init.lua`
    /// (or the user's HOME).
    #[test]
    fn user_init_lua_runs_after_bundled_plugin_set() {
        let layer = temp_layer("user-after-bundled");
        // Bundled plugin.
        write_plugin(
            &layer,
            "alpha.lua",
            r#"ttymap.register_palette_command({ label = "alpha", invoke = function() return true end })"#,
        );
        // "User" plugin (lives in the same layer for the test;
        // production would put this under
        // `~/.config/ttymap/lua/plugin/`).
        write_plugin(
            &layer,
            "user_beta.lua",
            r#"ttymap.register_palette_command({ label = "user-beta", invoke = function() return true end })"#,
        );
        // Bundled init.lua: standard order — bundled plugin first,
        // user config last.
        std::fs::write(
            layer.join("init.lua"),
            r#"
            require "plugin.alpha"
            require("ttymap.user_config").load()
            "#,
        )
        .expect("write bundled init.lua");

        let (lua, registry, _shared, _bus) = fresh_lua_layer_state(&layer);

        // Stub `ttymap.user_config.load()`: simulates a user
        // init.lua. At the point it runs, the bundled plugin's
        // palette entry must already exist — proving user runs
        // AFTER bundled. Then it requires a user plugin.
        lua.load(
            r#"
            package.preload["ttymap.user_config"] = function()
                return {
                    load = function()
                        _G.bundled_loaded_at_user_time =
                            package.loaded["plugin.alpha"] and 1 or 0
                        require "plugin.user_beta"
                    end,
                }
            end
            "#,
        )
        .exec()
        .expect("install user_config stub");

        let bundled_init = std::fs::read_to_string(layer.join("init.lua")).unwrap();
        lua.load(&bundled_init)
            .set_name("bundled-init")
            .exec()
            .expect("run bundled init");

        // Bundled was already loaded by the time user_config.load ran.
        let count: i64 = lua
            .globals()
            .get("bundled_loaded_at_user_time")
            .expect("global");
        assert_eq!(
            count, 1,
            "bundled plugin should have loaded BEFORE user config"
        );
        // Both entries land in registration order: bundled first,
        // then user.
        let labels: Vec<String> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect();
        assert_eq!(labels, vec!["alpha", "user-beta"]);
    }

    use std::path::Path;
}
