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

pub mod api;
pub mod bridge;
pub mod capture;
pub mod handle;
pub mod host;
pub mod init_lua;
pub mod map_api;
pub mod plugin_loader;
pub mod registrar;
pub mod runtimepath;
pub mod tick;
pub mod vm;

pub use bridge::palette_provider::LuaPaletteProvider;
pub use handle::LuaHandle;
pub use host::LuaHostShared;
pub use init_lua::read_init_lua_config_only;
pub use map_api::MapApi;
pub use registrar::{PluginRegistry, PluginRegistryHandle, new_plugin_registry};
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
    /// Live registry of plugin-declared activations + palette
    /// entries. Cloned into `BaseLayer` (for keypress dispatch) and
    /// the palette installer (for the `:` activation's per-open
    /// `CommandSeed` snapshot). Lua-side `PaletteCommandHandle:remove()`
    /// / `KeybindHandle:remove()` mutably borrow it to drop entries.
    pub registry: PluginRegistryHandle,
    /// `[<key> <name>]` footer hints harvested from the registry's
    /// palette entries at startup. Static for the program lifetime —
    /// adding / removing entries at runtime does not refresh this
    /// list (footer redraw avoidance trade-off; trivial to revisit
    /// if dynamic registration becomes a use case).
    pub plugin_hints: Vec<(&'static str, &'static str)>,
}

/// Build the Lua plugin subsystem: create the shared VM, install
/// the API surface, run the init.lua chain (which `require`s every
/// bundled plugin via the plugin-aware `package.searchers` entry,
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
/// 1. Build registry / slot / shared / ops / bus.
/// 2. `vm::new_lua()` — fresh VM with the lib searcher + extended
///    `package.path` for runtime layers.
/// 3. Install the `ttymap` global's pre-pass surface (`opt`,
///    `keymap`) so init.lua can mutate config defaults.
/// 4. `api::install` — extends `ttymap` with the runtime API
///    (`register_*`, `on_event`, `http`, `map`, …). Plugins need
///    these *during* their require, so this MUST run before init.lua.
/// 5. `vm::install_plugin_searcher` — inserts the plugin-aware
///    searcher at position 2 of `package.searchers`. Now any
///    top-level `require "<name>"` from init.lua resolves to a
///    `<layer>/plugin/<name>.lua` (or `<name>/init.lua`) and
///    attributions land under `<name>` in the registry.
/// 6. Run the init.lua chain (system → user). All plugin requires
///    fire here.
/// 7. `read_back` parses `ttymap.opt.*` into `Config`; clone
///    `KeybindingOverrides` from the keymap-state cell.
/// 8. Build the live `KeyMap` from overrides, fold its rows into
///    `shared.keymap_entries` so `ttymap.help:keymap_entries()` sees
///    them at render time. The brief register-time emptiness is
///    invisible — help.lua queries lazily.
/// 9. Harvest plugin hints, assemble [`LuaHandle`].
///
/// **Does not install the palette** — that step runs from the
/// composition root after this returns, draining `palette_entries`
/// into a default `:`-palette provider.
pub fn build_subsystem(defaults: Config) -> (LuaSubsystem, Config, KeybindingOverrides, KeyMap) {
    let registry = new_plugin_registry();
    let shared = Arc::new(LuaHostShared::new(defaults.geoip.endpoint.clone()));
    let bus = std::rc::Rc::new(crate::event::EventBus::default());
    let ops = op::new_ops_buffer();
    let slot = capture::new_capture_slot();

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
                    handle: LuaHandle::new(bus, Vec::new(), ops, shared),
                    registry,
                    plugin_hints: Vec::new(),
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
        slot.clone(),
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
                    handle: LuaHandle::new(bus, Vec::new(), ops, shared),
                    registry,
                    plugin_hints: Vec::new(),
                },
                defaults,
                KeybindingOverrides::new(),
                keymap,
            );
        }
    };

    // Now plugins requested via `require "<name>"` from init.lua get
    // resolved through the plugin-aware searcher and run via
    // `plugin_loader::register_one`, so their `register_*` /
    // `on_event` calls attribute under `<name>`.
    if let Err(e) = vm::install_plugin_searcher(
        &lua,
        runtime_path().to_vec(),
        slot.clone(),
        registry.clone(),
        shared.clone(),
    ) {
        log::warn!("lua: failed to install plugin searcher: {}", e);
    }

    // Run the bundled init.lua. It pulls in user config itself via
    // `require("ttymap.user_config").load()` (a Lua-side lib),
    // which keeps the user-config-path knowledge fully on the Lua
    // side — Rust never names `~/.config/ttymap/init.lua`. Errors
    // are logged + recovered.
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
    // remove. The footer slot is `[<key> <name>]`, built directly
    // from each entry's keybinding and `module.name`. No keybinding
    // ⇒ no footer slot.
    let plugin_hints: Vec<(&'static str, &'static str)> = registry
        .borrow()
        .palette_entries()
        .iter()
        .filter(|(_, e)| !e.hint.is_empty())
        .map(|(_, e)| {
            let key: &'static str = Box::leak(e.hint.clone().into_boxed_str());
            (key, e.name)
        })
        .collect();

    // Surface a one-shot warning if the user has files in
    // `~/.config/ttymap/plugin/` — that directory is no longer
    // auto-loaded; users must add `require "<name>"` lines to their
    // `init.lua`.
    warn_on_legacy_user_plugin_dir();

    let handle = LuaHandle::new(bus, vec![host_handles], ops, shared);

    (
        LuaSubsystem {
            handle,
            registry,
            plugin_hints,
        },
        config,
        keymap_overrides,
        keymap,
    )
}

/// Log a warning if `~/.config/ttymap/plugin/` exists and contains
/// `.lua` files. Auto-discovery there was removed; users must
/// `require` their plugins from `~/.config/ttymap/init.lua`.
fn warn_on_legacy_user_plugin_dir() {
    use directories::ProjectDirs;
    let Some(dirs) = ProjectDirs::from("", "", "ttymap") else {
        return;
    };
    let plugin_dir = dirs.config_dir().join("plugin");
    let Ok(entries) = std::fs::read_dir(&plugin_dir) else {
        return;
    };
    let has_lua = entries.filter_map(Result::ok).any(|e| {
        e.path()
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s == "lua")
            .unwrap_or(false)
    });
    if has_lua {
        log::warn!(
            "lua: {} is no longer auto-loaded. Add `require \"<name>\"` lines to ~/.config/ttymap/init.lua. See docs/lua-architecture.md.",
            plugin_dir.display()
        );
    }
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

    /// Helper for capture-inspection tests. Drives the host API
    /// install (so `ttymap.register_*` etc. exist) and runs the
    /// source through `load_chunk` directly — same path the
    /// plugin-aware searcher takes via `register_one`.
    fn parse_spec(
        source: &'static str,
        name: &'static str,
    ) -> (
        mlua::Lua,
        capture::CapturedRegistration,
        std::rc::Rc<crate::event::EventBus>,
    ) {
        let lua = new_lua();
        let slot = capture::new_capture_slot();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        api::install(
            &lua,
            host::LuaHostShared::empty(),
            slot.clone(),
            op::new_ops_buffer(),
            bus.clone(),
            new_plugin_registry(),
        )
        .expect("install ttymap");
        let captured = bridge::handle::load_chunk(&lua, source, name, &slot).expect("load chunk");
        (lua, captured, bus)
    }

    /// Build a Lua VM with the plugin-aware searcher pointed at a
    /// custom runtime layer. Returns `(lua, registry, shared, bus)`
    /// so tests can drive `lua.load(r#"require '<name>'"#).exec()`
    /// and assert what landed in the registry / shared snapshot.
    fn fresh_pluginsearcher_state(
        layer: &Path,
    ) -> (
        mlua::Lua,
        PluginRegistryHandle,
        Arc<host::LuaHostShared>,
        std::rc::Rc<crate::event::EventBus>,
    ) {
        let lua = new_lua();
        // Extend package.path so lib requires resolve from the custom
        // layer, mirroring the production setup `vm::new_lua` does for
        // the global runtime path. `plugin/` is NOT added here — the
        // plugin searcher (installed below) owns that tree.
        vm::prepend_package_path(&lua, &layer.join("lua"));
        let shared = host::LuaHostShared::empty();
        let slot = capture::new_capture_slot();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let registry = new_plugin_registry();
        api::install(
            &lua,
            shared.clone(),
            slot.clone(),
            op::new_ops_buffer(),
            bus.clone(),
            registry.clone(),
        )
        .expect("install ttymap");
        vm::install_plugin_searcher(
            &lua,
            vec![layer.to_path_buf()],
            slot,
            registry.clone(),
            shared.clone(),
        )
        .expect("install plugin searcher");
        (lua, registry, shared, bus)
    }

    /// Build a private temp directory rooted at the OS's temp dir.
    /// `unique` should differ per test so parallel runs don't
    /// stomp on each other.
    fn temp_layer(unique: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ttymap-lua-test-{}", unique));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugin")).expect("mkdir plugin");
        std::fs::create_dir_all(dir.join("lua")).expect("mkdir lua");
        dir
    }

    fn write_plugin(layer: &Path, file_name: &str, lua: &str) {
        std::fs::write(layer.join("plugin").join(file_name), lua).expect("write plugin");
    }

    fn write_plugin_dir(layer: &Path, name: &str, lua: &str) {
        let dir = layer.join("plugin").join(name);
        std::fs::create_dir_all(&dir).expect("mkdir plugin dir");
        std::fs::write(dir.join("init.lua"), lua).expect("write plugin init.lua");
    }

    fn labels(r: &PluginRegistryHandle) -> Vec<String> {
        r.borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.clone())
            .collect()
    }

    #[test]
    fn explicit_palette_command_and_keybind_are_captured() {
        let (_lua, c, _bus) = parse_spec(
            r#"
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
    fn on_tick_subscriptions_land_on_the_bus() {
        // Each `ttymap.api.frame.on_tick(fn)` call subscribes
        // directly against the bus and bumps `events_registered`.
        // Combined with at least one activation surface so the load
        // passes the "must subscribe to something" gate.
        let (_lua, c, bus) = parse_spec(
            r#"
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.register_palette_command({ label = "x", invoke = function() end })
            "#,
            "x",
        );
        assert_eq!(c.events_registered, 2);
        assert_eq!(bus.count("tick"), 2);
    }

    #[test]
    fn script_with_only_on_tick_passes_subscription_gate() {
        // A script that only subscribes via `on_tick` (no palette
        // command, no keybind) is a valid plugin — always-on chrome
        // like info / scalebar fits this shape.
        let (_lua, c, bus) =
            parse_spec(r#"ttymap.api.frame.on_tick(function(_map) end)"#, "chrome");
        assert!(c.palette_commands.is_empty());
        assert!(c.keybinds.is_empty());
        assert_eq!(c.events_registered, 1);
        assert_eq!(bus.count("tick"), 1);
    }

    #[test]
    fn init_lua_require_attributes_plugin_correctly() {
        // `require "demo"` from init.lua-equivalent driver must
        // route through the plugin searcher's wrapper, set
        // current_plugin = "demo" via load_chunk, drain captures
        // into the registry under name "demo", and surface help
        // metadata in the shared palette_entries snapshot.
        let layer = temp_layer("require-attribution");
        write_plugin(
            &layer,
            "demo.lua",
            r#"
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.register_palette_command({ label = "Toggle demo", invoke = function() return true end })
            ttymap.register_keybind("d", function() return true end)
            "#,
        );

        let (lua, registry, shared, _bus) = fresh_pluginsearcher_state(&layer);
        lua.load(r#"require "demo""#).exec().expect("require demo");

        // Registry entry attributed under "demo".
        let names: Vec<&'static str> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.name)
            .collect();
        assert!(
            names.contains(&"demo"),
            "plugin should attribute under its require name, got {:?}",
            names
        );

        // Shared snapshot for help.
        let entries = shared.palette_entries.lock().expect("lock palette_entries");
        let demo = entries
            .iter()
            .find(|e| e.name == "demo")
            .expect("demo plugin should be in snapshot");
        assert_eq!(demo.key, "d");
        assert_eq!(demo.label, "Toggle demo");
    }

    #[test]
    fn directory_plugin_is_required_via_init_lua() {
        // `<layer>/plugin/wiki/init.lua` is the plugin entry; user
        // does `require "wiki"`. Mirrors a directory-shaped bundled
        // plugin (e.g. travel/, satellite/).
        let layer = temp_layer("require-dir-plugin");
        write_plugin_dir(
            &layer,
            "biggie",
            r#"ttymap.register_palette_command({ label = "biggie", invoke = function() return true end })"#,
        );

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);
        lua.load(r#"require "biggie""#)
            .exec()
            .expect("require biggie");
        let ls = labels(&registry);
        assert!(
            ls.iter().any(|l| l.contains("biggie")),
            "expected biggie/init.lua to register, got {:?}",
            ls
        );
    }

    #[test]
    fn lib_require_does_not_attribute_as_plugin() {
        // A lib at `lua/ttymap/foo.lua` is required via
        // `require "ttymap.foo"`. Dotted name → plugin searcher
        // skips. Resolves via package.path → loads as plain chunk.
        // No registration happens (libs don't call register_*).
        let layer = temp_layer("lib-require");
        std::fs::create_dir_all(layer.join("lua").join("ttymap")).expect("mkdir lua/ttymap");
        std::fs::write(
            layer.join("lua").join("ttymap").join("foo.lua"),
            r#"return { value = 42 }"#,
        )
        .expect("write lib");

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);
        let v: i64 = lua
            .load(r#"return require("ttymap.foo").value"#)
            .eval()
            .expect("require ttymap.foo");
        assert_eq!(v, 42);
        // Lib never goes through the plugin wrapper, so registry stays empty.
        assert_eq!(registry.borrow().palette_entry_count(), 0);
    }

    #[test]
    fn requiring_same_plugin_twice_only_fires_once() {
        // Lua's `package.loaded` cache: after the first require runs
        // the plugin wrapper, subsequent `require "demo"` calls return
        // the cached value (true, since wrapper returns nil) without
        // re-executing the wrapper. So system + user init.lua both
        // doing `require "demo"` register exactly one entry.
        let layer = temp_layer("dedup");
        write_plugin(
            &layer,
            "demo.lua",
            r#"ttymap.register_palette_command({ label = "demo", invoke = function() return true end })"#,
        );

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);
        lua.load(r#"require "demo"; require "demo""#)
            .exec()
            .expect("double require");

        assert_eq!(
            registry.borrow().palette_entry_count(),
            1,
            "duplicate require must not double-register"
        );
    }

    #[test]
    fn submodule_require_skips_plugin_wrap() {
        // `require "travel.routes.italy"` from inside a plugin must
        // resolve as a plain chunk via package.path's
        // `<layer>/plugin/?.lua` — NOT through the plugin wrapper
        // (which would register "travel.routes.italy" as a fake
        // plugin). Only the top-level `travel` require triggers the
        // wrapper.
        let layer = temp_layer("submodule");
        let travel_dir = layer.join("plugin").join("travel");
        std::fs::create_dir_all(travel_dir.join("routes")).expect("mkdir routes");
        std::fs::write(
            travel_dir.join("routes").join("italy.lua"),
            r#"return { city = "Rome" }"#,
        )
        .expect("write submodule");
        std::fs::write(
            travel_dir.join("init.lua"),
            r#"
            local italy = require "travel.routes.italy"
            ttymap.register_palette_command({
                label = italy.city,
                invoke = function() return true end,
            })
            "#,
        )
        .expect("write travel/init.lua");

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);
        lua.load(r#"require "travel""#)
            .exec()
            .expect("require travel");
        let names: Vec<&'static str> = registry
            .borrow()
            .palette_entries()
            .iter()
            .map(|(_, e)| e.name)
            .collect();
        // Only one entry, attributed to "travel" — the submodule did
        // not become a plugin under name "travel.routes.italy".
        assert_eq!(names, vec!["travel"]);
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

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);
        // init.lua-style pre-pass: mutate the lib's table BEFORE the
        // plugin requires it. Lua's module cache makes the plugin's
        // `require "ttymap.myplug"` return the same table.
        lua.load(r#"require("ttymap.myplug").label = "from-init""#)
            .exec()
            .expect("init pre-pass");
        // Now activate the plugin; it should pick up the mutated
        // label.
        lua.load(r#"require "myplug""#)
            .exec()
            .expect("require myplug");

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
    /// `require("ttymap.user_config").load()`. User init.lua should
    /// activate user plugins via `require` (same path as bundled —
    /// the searcher wrapper drains the slot per plugin); calling
    /// `register_*` directly from init.lua isn't supported because
    /// nothing drains the slot for the bare init chunk.
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
        // production would put this under `~/.config/ttymap/plugin/`).
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
            require "alpha"
            require("ttymap.user_config").load()
            "#,
        )
        .expect("write bundled init.lua");

        let (lua, registry, _shared, _bus) = fresh_pluginsearcher_state(&layer);

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
                            package.loaded.alpha and 1 or 0
                        require "user_beta"
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
