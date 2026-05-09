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
pub mod loader;
pub mod map_api;
pub mod registrar;
pub mod runtimepath;
pub mod tick;
pub mod vm;

pub use bridge::palette_provider::LuaPaletteProvider;
pub use handle::LuaHandle;
pub use host::LuaHostShared;
pub use init_lua::load_init_lua;
pub use map_api::MapApi;
pub use registrar::Registrar;
pub use runtimepath::{resolve_runtime_path, runtime_path, set_runtime_path};
pub use vm::new_lua;

use std::sync::Arc;

use crate::UserCommand;
use crate::compositor::op;
use crate::compositor::{Activation, PaletteEntry};
use crate::config::Config;
use crate::input::KeyMap;

/// Result of [`build_subsystem`].
///
/// The runtime-held [`LuaHandle`] is **already constructed** by
/// `build_subsystem` itself — the App just stores it, no longer
/// reaches into a registrar to assemble it. The remaining fields are
/// the parts that flow into the [`crate::compositor`] (activations,
/// plugin_hints) and the palette installer (palette_entries).
pub struct LuaSubsystem {
    /// Runtime handle to the Lua subsystem — semantic surface
    /// App uses to observe state changes and tick plugins.
    pub handle: LuaHandle,
    /// Per-plugin keymap activations (`<key>` ⇒ spawn component).
    /// Consumed by the compositor's `BaseLayer`.
    pub activations: Vec<Activation>,
    /// Plugin-supplied palette entries — drained by
    /// [`crate::palette::install`] in `main` before the
    /// rest of this struct reaches `App::new`.
    pub palette_entries: Vec<PaletteEntry>,
    /// `[<key> <name>]` footer hints harvested from
    /// `palette_entries` *before* the palette installer drains them.
    pub plugin_hints: Vec<(&'static str, &'static str)>,
}

/// Build the Lua plugin subsystem: load every `*.lua` under the
/// runtime path's `plugin/` layers in the **shared** Lua VM
/// (`lua` argument — the same VM `init.lua` already ran in via
/// [`load_init_lua`]), register their activations / palette entries
/// / event-bus subscriptions, and assemble the runtime [`LuaHandle`].
///
/// nvim-style: a single Lua state hosts init.lua, every bundled
/// plugin, and every user plugin. `init.lua`'s `require "ttymap.<X>"`
/// returns the same module a plugin gets later, so per-plugin config
/// holders (`runtime/lua/ttymap/<plugin>.lua`) work the natural way:
/// init.lua mutates the lib's table, the plugin reads the mutated
/// values when it loads.
///
/// **Does not install the palette** — that step runs from the
/// composition root after this returns, draining `palette_entries`
/// into a default `:`-palette provider.
pub fn build_subsystem(
    lua: mlua::Lua,
    config: &Config,
    attribution: Option<String>,
    keymap: &KeyMap,
) -> LuaSubsystem {
    let mut r = Registrar::default();

    // Build the shared runtime-data carrier once. Every Lua plugin
    // (bundled and user) sees the same `ttymap.*` accessor surface;
    // there is no per-plugin Rust glue, no per-plugin upvalue
    // injection. Adding a new bundled plugin is one file under
    // `runtime/plugin/`; adding a user plugin is one file in
    // `~/.config/ttymap/plugin/`.
    let shared = Arc::new(LuaHostShared::new(
        attribution,
        config.geoip.endpoint.clone(),
        keymap_entries(keymap),
    ));

    // Build the event bus first — it's shared between the API
    // surface (so `on_event` / `on_tick` can subscribe directly and
    // return an `EventHandle` carrying back a clone) and the
    // [`LuaHandle`] (so App publishes go through the same bus
    // every plugin subscribed against).
    let bus = std::rc::Rc::new(crate::event::EventBus::default());

    // Extend the existing `ttymap` global (created by init.lua's
    // pre-pass with `opt` + `keymap`) with the plugin runtime API:
    // `http`, `map`, `api`, `register_*`, `notify`, `on_event`. One
    // install for the whole subsystem; every plugin sees the same
    // `ttymap` table.
    let ops = op::new_ops_buffer();
    let slot = capture::new_capture_slot();
    let host_handles =
        match api::install(&lua, shared.clone(), slot.clone(), ops.clone(), bus.clone()) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("lua: api::install failed: {} — plugins disabled", e);
                // Build an empty handle so the App still boots.
                return LuaSubsystem {
                    handle: LuaHandle::new(bus, Vec::new(), ops, shared),
                    activations: Vec::new(),
                    palette_entries: Vec::new(),
                    plugin_hints: Vec::new(),
                };
            }
        };

    // Bundled plugins (every `*.lua` under each runtime layer's
    // `plugin/`) always register — disabling one is either an
    // entry in `ttymap.opt.disable` (skipped at the walker) or a
    // higher-priority runtime layer shadowing it by stem.
    loader::register_builtin_plugins(
        &lua,
        &slot,
        runtime_path(),
        &config.plugins.disable,
        shared.clone(),
        &mut r,
    );

    // Harvest the BaseLayer's footer hints. Has to happen *before*
    // the palette installer drains `palette_entries`. The footer
    // slot is `[<key> <name>]` — built directly from each entry's
    // keybinding and `module.name`. No keybinding ⇒ no footer slot.
    let plugin_hints: Vec<(&'static str, &'static str)> = r
        .palette_entries
        .iter()
        .filter(|e| !e.hint.is_empty())
        .map(|e| {
            let key: &'static str = Box::leak(e.hint.clone().into_boxed_str());
            (key, e.name)
        })
        .collect();

    // Wrap the shared bus + the single shared host handles into the
    // [`LuaHandle`], so callers never see the bus or the channels
    // and never have to assemble a handle themselves. `shared` lives
    // on as the handle's own clone — App writes into shared cells
    // (e.g. `current_frame`) through `LuaHandle` semantic methods.
    let handle = LuaHandle::new(bus, vec![host_handles], ops, shared);

    LuaSubsystem {
        handle,
        activations: r.activations,
        palette_entries: r.palette_entries,
        plugin_hints,
    }
}

/// Build the `(key-binding, action-label)` pairs that the help
/// plugin surfaces via `ttymap.help:keymap_entries()`. Live data —
/// runtime keymap overrides surface here.
fn keymap_entries(keymap: &KeyMap) -> Vec<(String, String)> {
    UserCommand::keymap_help_entries(keymap)
}

#[cfg(test)]
mod tests {
    use super::loader::register_plugins_in;
    use super::vm::{install_builtin_searcher, prepend_package_path};
    use super::*;
    use mlua::{Lua, Result};
    use std::path::Path;

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
    /// some registrar slot (palette / key bind / tick) — the
    /// dispatcher just routes whatever the script subscribes to,
    /// so this round-trips the parse + meta + wire path. Set-
    /// membership rather than counts so adding a builtin doesn't
    /// require updating magic numbers.
    #[test]
    fn every_bundled_script_registers() {
        runtimepath::ensure_runtime_path_for_tests();
        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        let rtp = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime")];
        let (lua, slot, bus) = fresh_loader_state(shared.clone());
        loader::register_builtin_plugins(&lua, &slot, &rtp, &[], shared, &mut r);

        let palette: std::collections::HashSet<String> = r
            .palette_entries
            .iter()
            .map(|e| e.label.to_lowercase())
            .collect();
        // info / scalebar / attribution always subscribe a tick
        // (always-on chrome). Panel migrations (aircraft / quake /
        // wiki / search / here / satellite) subscribe a tick for
        // async drain + paint. Toggle-on-demand plugins (center,
        // terminator) only subscribe when active, so they don't
        // count here. Lower bound rather than equality so adding a
        // builtin doesn't require updating a magic number.
        let tick_count = bus.count("tick");

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
        assert!(
            tick_count >= 4,
            "info/scalebar/attribution/center should each subscribe a tick callback (got {tick_count})"
        );
    }

    /// Helper for capture inspection tests. Runs the source in a
    /// throwaway Lua state and returns the captured registration so
    /// the test can assert on the activation surfaces and tick
    /// subscriptions. Mirrors what `register_one` does at
    /// registration time. The bus is returned alongside so tests
    /// can assert against `on_event` / `on_tick` subscriptions
    /// (those subscribe directly to the bus rather than into the
    /// capture slot).
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
        )
        .expect("install ttymap");
        let captured = bridge::handle::load_chunk(&lua, source, name, &slot).expect("load chunk");
        (lua, captured, bus)
    }

    /// Helper for `register_plugins_in`-driven tests. Builds a fresh
    /// Lua state with the runtime API installed and returns the
    /// `(lua, slot, bus)` triple the dir-discovery tests pass into
    /// the loader.
    fn fresh_loader_state(
        shared: Arc<host::LuaHostShared>,
    ) -> (
        mlua::Lua,
        capture::CaptureSlot,
        std::rc::Rc<crate::event::EventBus>,
    ) {
        let lua = new_lua();
        let slot = capture::new_capture_slot();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        api::install(
            &lua,
            shared,
            slot.clone(),
            op::new_ops_buffer(),
            bus.clone(),
        )
        .expect("install ttymap");
        (lua, slot, bus)
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
    fn registering_a_plugin_publishes_metadata() {
        // Round-trip the registration path: a Lua source declaring
        // `key` + `label` should land in the shared snapshot help
        // reads via `ttymap.help:palette_entries()`. The plugin's
        // identity is the file stem (`demo`), passed to the registrar
        // by the walker.
        let dir = temp_plugins_dir("publish-meta");
        write_plugin(
            &dir,
            "demo.lua",
            r#"
            ttymap.api.frame.on_tick(function(_map) end)
            ttymap.register_palette_command({ label = "Toggle demo", invoke = function() return true end })
            ttymap.register_keybind("d", function() return true end)
            "#,
        );

        let shared = host::LuaHostShared::empty();
        let mut r = Registrar::default();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared.clone(), &mut r);

        let entries = shared.palette_entries.lock().expect("lock palette_entries");
        let demo = entries
            .iter()
            .find(|e| e.name == "demo")
            .expect("demo plugin should be in snapshot");
        assert_eq!(demo.key, "d");
        assert_eq!(demo.label, "Toggle demo");
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
            r#"ttymap.register_palette_command({ label = "first", invoke = function() return true end })"#,
        );
        write_plugin(
            &dir,
            "second.lua",
            r#"ttymap.register_palette_command({ label = "second", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
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
            r#"ttymap.register_palette_command({ label = "ok", invoke = function() return true end })"#,
        );
        // README, backup files, etc. should be ignored.
        std::fs::write(dir.join("README.md"), "ignore me").unwrap();
        std::fs::write(dir.join("ok.lua.bak"), "ignore me too").unwrap();

        let mut r = Registrar::default();
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
        let ls = labels(&r);
        assert_eq!(ls.len(), 1, "got {:?}", ls);
        assert!(ls[0].contains("ok"));
    }

    #[test]
    fn dir_discovery_honours_opt_disable_list() {
        // `ttymap.opt.disable = { "beta" }` — the walker skips the
        // matching stem at registration time.
        let dir = temp_plugins_dir("opt-disable");
        write_plugin(
            &dir,
            "alpha.lua",
            r#"ttymap.register_palette_command({ label = "alpha", invoke = function() return true end })"#,
        );
        write_plugin(
            &dir,
            "beta.lua",
            r#"ttymap.register_palette_command({ label = "beta", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        let disable = vec!["beta".to_string()];
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &disable, shared, &mut r);
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
            r#"ttymap.register_palette_command({ label = "biggie", invoke = function() return true end })"#,
        )
        .expect("write init.lua");

        let mut r = Registrar::default();
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
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
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
        assert!(
            !labels(&r).iter().any(|l| l.contains("libonly")),
            "lib-only subdir must not register as a plugin"
        );
    }

    #[test]
    fn dir_discovery_skips_broken_lua_but_keeps_going() {
        let dir = temp_plugins_dir("broken");
        write_plugin(&dir, "broken.lua", "this is not lua syntax !!!");
        write_plugin(
            &dir,
            "ok.lua",
            r#"ttymap.register_palette_command({ label = "ok", invoke = function() return true end })"#,
        );

        let mut r = Registrar::default();
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
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
        let shared = host::LuaHostShared::empty();
        let (lua, slot, _bus) = fresh_loader_state(shared.clone());
        register_plugins_in(&lua, &slot, &dir, None, &[], shared, &mut r);
        assert!(r.palette_entries.is_empty());
    }

    /// The whole point of unifying init.lua's VM with the plugin VM:
    /// a config-holder lib at `runtime/lua/<name>.lua` is reachable
    /// via `require` from BOTH init.lua AND a plugin script, and
    /// both round-trips return the same cached table — so a user's
    /// init.lua can pre-mutate the table and the plugin sees the
    /// mutation when it loads later. Pre-unification each side held
    /// its own VM and `require` returned a different copy per VM,
    /// so this only "worked" via whole-file shadow.
    /// The whole point of unifying init.lua's VM with the plugin VM:
    /// a config-holder lib at `runtime/lua/ttymap/<name>.lua` is
    /// reachable via `require "ttymap.<name>"` from BOTH init.lua
    /// AND a plugin script, and both round-trips return the same
    /// cached table — so a user's init.lua can pre-mutate the table
    /// and the plugin sees the mutation when it loads later. Pre-
    /// unification each side held its own VM and `require` returned
    /// a different copy per VM, so this only "worked" via whole-file
    /// shadow.
    #[test]
    fn init_lua_can_seed_config_for_a_plugin_via_require() {
        // Custom runtime layer with a config holder + a plugin that
        // reads it. The lib at `lua/ttymap/myplug.lua` exposes a
        // defaults table; the plugin at `plugin/myplug.lua` reads
        // `cfg.label` at load time and uses it as its palette
        // command label.
        let layer = temp_plugins_dir("init-seeds-plugin-config");
        std::fs::create_dir_all(layer.join("lua").join("ttymap")).expect("mkdir lua/ttymap");
        std::fs::create_dir_all(layer.join("plugin")).expect("mkdir plugin");
        std::fs::write(
            layer.join("lua").join("ttymap").join("myplug.lua"),
            "return { label = 'default-label' }",
        )
        .expect("write lib");
        std::fs::write(
            layer.join("plugin").join("myplug.lua"),
            r#"
            local cfg = require "ttymap.myplug"
            ttymap.register_palette_command({
                label = cfg.label,
                invoke = function() return true end,
            })
            "#,
        )
        .expect("write plugin");

        // init.lua-style pre-pass: same VM, mutates the lib's table
        // before the plugin runs. The Lua cache makes the plugin's
        // `require "ttymap.myplug"` return the same table. We
        // configure `package.path` directly rather than touching the
        // global runtime path — multiple tests share the global
        // and a parallel run could otherwise step on each other.
        let lua = mlua::Lua::new();
        vm::prepend_package_path(&lua, &layer.join("lua"));
        lua.load(r#"require("ttymap.myplug").label = "from-init""#)
            .exec()
            .expect("init pre-pass");

        let shared = host::LuaHostShared::empty();
        let slot = capture::new_capture_slot();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        api::install(
            &lua,
            shared.clone(),
            slot.clone(),
            op::new_ops_buffer(),
            bus,
        )
        .expect("install ttymap");
        let mut r = Registrar::default();
        loader::register_builtin_plugins(&lua, &slot, &[layer], &[], shared, &mut r);

        let labels: Vec<String> = r.palette_entries.iter().map(|e| e.label.clone()).collect();
        assert!(
            labels.iter().any(|l| l == "from-init"),
            "plugin should read the mutated lib's `label`, got {:?}",
            labels,
        );
    }
}
