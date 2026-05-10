//! Builder for the runtime `ttymap` Lua global — the API surface every
//! plugin script reaches into.
//!
//! `ttymap` is a Lua **table** (not a single userdata) whose fields
//! are domain-namespaced userdatas. Each namespace owns the slice of
//! state its methods need; nothing forces every plugin's call to walk
//! a kitchen-sink struct. Adding a new domain (orbit propagation,
//! logging, scheduling, …) is one new namespace, no churn on existing
//! ones.
//!
//! Submodules: one per Lua namespace (`ttymap.<X>`).
//! - [`http`], [`json`], [`sgp4`] — top-level userdata namespaces
//! - [`map`] — `ttymap.map` userdata (`HostMap`) **and** the per-frame
//!   `map` table handed to `on_tick` callbacks (`make_map_table`,
//!   wrapping the host-side [`crate::lua::MapApi`])
//! - `config`, `help`, `log`, `tile` — host-state namespaces
//! - `imperative` — `ttymap.api.{card,palette,frame}` cluster
//! - `register` — setup-time `ttymap.register_*` / `on_event` capture
//!
//! Surface today:
//!
//! ```text
//! ttymap.http   :fetch(url) -> Job          background HTTP GET (UTF-8 body).
//!                                            Job: :try_take() polls; :cancel()
//!                                            disposes (idempotent — buffered
//!                                            body becomes unreachable from
//!                                            try_take after cancel).
//! ttymap.http   :fetch_cached(url, ttl) -> Job  disk-cached GET; on HTTP
//!                                            error falls back to the
//!                                            stale on-disk copy if any
//! ttymap.http   :url_encode(s) -> string    RFC 3986 query encoding
//! ttymap.map    :jump(lon, lat)             recentre the map (fire-and-forget)
//! ttymap.map    :zoom(level)                set zoom directly (clamped to map's
//!                                            allowed range; fire-and-forget)
//! ttymap.map    :zoom() -> level             current zoom (no-arg getter form),
//!                                            refreshed per dispatch
//! ttymap.map    :fly_to(lon, lat, zoom)     composite recenter + zoom in one
//!                                            dispatch (avoids the intermediate
//!                                            new-centre / old-zoom frame)
//! ttymap.map    :center() -> lon, lat       latest centre, refreshed per dispatch
//! ttymap.json   :parse(s) -> value|nil      JSON → Lua tables (errors → nil)
//! ttymap.sgp4   :parse_tle(text) -> handle  parse a TLE for SGP4 propagation
//! ttymap.sgp4   :parse_tles(text) -> array  parse a multi-TLE block (groups)
//! ttymap.sgp4   :propagate(h[, t]) -> table propagate a handle to unix time t
//! ttymap.sgp4   :propagate_batch(hs[, t])   batch propagate (Starlink-scale)
//! ttymap.tile   :attribution() -> string?   active tile provider's attribution
//! ttymap.config :geoip_endpoint() -> string `[geoip].endpoint` value
//! ttymap.help   :keymap_entries() -> list   built-in keymap rows for help
//! ttymap.help   :palette_entries() -> list  per-plugin metadata for help
//! ttymap.log    :info(msg) / :warn(msg) / :error(msg)
//!                                            forward to host log at
//!                                            target `lua`
//! ttymap.api.card.open(spec) -> Handle    push a focused window
//!                                            (LuaCardComponent) onto
//!                                            the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.palette.open(spec) -> Handle   push a palette provider
//!                                            onto the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.frame.to_ansi() -> string?    latest frame as ANSI bytes,
//!                                            or nil if no frame yet (caller
//!                                            decides where to persist)
//! ttymap.api.frame.on_tick(callback)        register a per-frame callback
//!                                            (called with `MapApi`); multiple
//!                                            calls per script are stacked
//! ttymap.notify(msg [, opts])               post a transient status message;
//!                                            opts.level is `info` (default) /
//!                                            `warn` / `error`. Lowers to
//!                                            `Op::Publish(Event::Notify)`;
//!                                            the bundled `notify.lua`
//!                                            subscriber renders recent
//!                                            entries in a corner.
//! ttymap.on_event("notify", fn)              subscribe to bus events;
//!                                            `notify` payload is
//!                                            `{ message, level }` table.
//! ```
//!
//! `ttymap.map:jump(...)` is fire-and-forget from the Lua side; the
//! matching `Receiver` on the App drains after each setup-state
//! callback. `ttymap.map:center()` reads a `Mutex<LonLat>` the
//! component refreshes at the start of every dispatch path that
//! carries a `Window` / `MapApi`, so callers see the latest centre
//! without threading anything through their signatures.
//!
//! `ttymap` is a Neovim-style **single-VM global**: `init.lua`
//! installs `ttymap.opt` / `ttymap.keymap` first; this module then
//! adds `ttymap.http` / `map` / `api` / `register_*` / `notify` /
//! `on_event` to the same table before plugins load. Every plugin
//! sees one unified `ttymap` namespace, and `init.lua` can
//! `require "ttymap.<plugin_name>"` and mutate a config table the
//! plugin will read later (Lua's module cache makes the require
//! return the same table on the plugin side).

pub mod http;
pub mod json;
pub mod map;
pub mod sgp4;

mod config;
mod help;
mod imperative;
mod log;
mod register;
mod tile;

use config::HostConfig;
use help::HostHelp;
use log::HostLog;
use map::HostMap;
use tile::HostTile;

use std::sync::{Arc, Mutex};

use mlua::{Lua, Table};

use crate::compositor::op::Op;
use crate::event::{Event, Level};
use crate::lua::host::{LuaHostHandles, LuaHostShared};
use ttymap_engine::geo::LonLat;
use ttymap_engine::shared::http::HttpClient;

// ── Install entry point ─────────────────────────────────────────────

/// Extend the `ttymap` global with the plugin runtime API
/// (`http` / `map` / `api` / `register_*` / `notify` / `on_event` …)
/// and return the host handles plugins read view state through.
///
/// The `ttymap` global must already exist — `init.lua`'s pre-pass
/// (see [`crate::lua::init_lua`]) creates it with `opt` / `keymap`
/// before this runs. We add fields to the same table rather than
/// replace it, so `ttymap.opt.*` mutations from `init.lua` survive.
///
/// "Plugin" is purely a Lua-side concept — a `.lua` file's worth of
/// `register_palette_command` / `register_keybind` / `on_event`
/// calls. The host has no notion of plugin identity, no per-script
/// slot, no attribution. Each `register_*` call pushes directly into
/// `registry` and returns a handle to Lua.
pub fn install(
    lua: &Lua,
    shared: Arc<LuaHostShared>,
    ops: crate::compositor::op::OpsBuffer,
    bus: std::rc::Rc<crate::event::EventBus>,
    registry: crate::lua::registrar::LuaRegistryHandle,
) -> mlua::Result<LuaHostHandles> {
    // Fire-and-forget Lua intents (`map:jump`, `:zoom`, `:fly_to`,
    // `frame.export`) enqueue `Op::Command(UserCommand::...)` onto
    // `ops`; the App drains and dispatches per iteration alongside
    // every other source. Plugin trust model is nvim-style (anything
    // the user could do, a plugin can also do).
    let center = Arc::new(Mutex::new(LonLat { lon: 0.0, lat: 0.0 }));
    let zoom = Arc::new(Mutex::new(0.0_f64));

    let ttymap: Table = match lua.globals().get::<mlua::Value>("ttymap")? {
        mlua::Value::Table(t) => t,
        _ => {
            // No init pre-pass ran (test paths or future callers). Create
            // an empty `ttymap` so the rest of install can proceed.
            let t = lua.create_table()?;
            lua.globals().set("ttymap", t.clone())?;
            t
        }
    };

    ttymap.set(
        "http",
        lua.create_userdata(http::HostHttp {
            http: HttpClient::new("lua").map_err(mlua::Error::external)?,
        })?,
    )?;
    ttymap.set(
        "map",
        lua.create_userdata(HostMap::new(ops.clone(), center.clone(), zoom.clone()))?,
    )?;
    ttymap.set("json", lua.create_userdata(json::HostJson)?)?;
    ttymap.set("sgp4", lua.create_userdata(sgp4::HostSgp4)?)?;
    ttymap.set("tile", lua.create_userdata(HostTile::new(shared.clone()))?)?;
    ttymap.set(
        "config",
        lua.create_userdata(HostConfig::new(shared.clone()))?,
    )?;
    ttymap.set("help", lua.create_userdata(HostHelp::new(shared.clone()))?)?;
    ttymap.set("log", lua.create_userdata(HostLog::new("lua".to_string()))?)?;

    // Activation surfaces (`register_palette_command` /
    // `register_keybind` / `on_event`) — every call pushes directly
    // into the live `LuaRegistry` (or, for `on_event`, subscribes
    // directly against the bus) and returns a Lua-facing handle.
    // No deferred capture, no per-script slot.
    register::install(lua, &ttymap, bus.clone(), registry, shared.clone())?;

    // Imperative primitives (`ttymap.api.{card,palette,frame}`) —
    // runtime-time `open` / `to_ansi` / `on_tick` calls a plugin
    // makes from inside its callbacks.
    imperative::install(lua, &ttymap, ops.clone(), shared.clone(), bus)?;

    // ── ttymap.notify ────────────────────────────────────────────────
    //
    // Top-level write surface for transient status messages. Kept as
    // a plain function (not method-style) so callers write
    // `ttymap.notify("ok")` instead of `ttymap.notify:post("ok")` —
    // the call site is the common one. The notification rides the
    // shared `OpsBuffer` as `Op::Publish(Event::Notify {...})`; the
    // App-side dispatcher hands it to `EventBus::publish`, which
    // fans out to whoever subscribed (today: bundled `notify.lua`).
    let ops_for_notify = ops;
    ttymap.set(
        "notify",
        lua.create_function(move |_, (msg, opts): (String, Option<Table>)| {
            let level = opts
                .and_then(|t| t.get::<String>("level").ok())
                .map(|s| Level::parse(&s))
                .unwrap_or(Level::Info);
            ops_for_notify.borrow_mut().push(Op::Publish(Event::Notify {
                message: msg,
                level,
            }));
            Ok(())
        })?,
    )?;

    // ── ttymap.runtime_path ──────────────────────────────────────────
    //
    // The resolved runtime layer list as a 1-indexed Lua array. The
    // bundled `ttymap.plugin_searcher` Lua lib reads this to walk
    // `<layer>/plugin/...` for plugin require resolution. Rust never
    // names the `plugin/` subdirectory itself — that string lives
    // entirely in the Lua searcher.
    let layers: Vec<String> = crate::lua::runtime_path()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    ttymap.set("runtime_path", lua.create_sequence_from(layers)?)?;

    Ok(LuaHostHandles { center, zoom })
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserCommand;
    use ttymap_engine::map::MapAction;

    /// Helper for tests: install the `ttymap` table into a fresh Lua
    /// and hand back the host handles + the shared op buffer. Mirrors
    /// the production install path; the bus is dropped since these
    /// tests don't exercise registration or dispatch.
    fn install_for_test() -> (mlua::Lua, LuaHostHandles, crate::compositor::op::OpsBuffer) {
        let lua = mlua::Lua::new();
        let ops = crate::compositor::op::new_ops_buffer();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let registry = crate::lua::new_lua_registry();
        let handles = install(&lua, LuaHostShared::empty(), ops.clone(), bus, registry)
            .expect("install ttymap table");
        (lua, handles, ops)
    }

    #[test]
    fn ttymap_table_is_installed_with_namespaces() {
        let (lua, _handles, _ops) = install_for_test();
        // Each namespace lookup must return a userdata; the shape
        // confirms the install wired all namespaces in.
        for ns in [
            "http", "map", "json", "sgp4", "tile", "config", "help", "log",
        ] {
            let ud: mlua::AnyUserData = lua
                .load(format!("return ttymap.{ns}"))
                .eval()
                .unwrap_or_else(|e| panic!("ttymap.{ns} should be a userdata: {e}"));
            // Just confirm round-trip works.
            let _ = ud;
        }
    }

    #[test]
    fn host_map_jump_pushes_appmsg_jump() {
        // `ttymap.map:jump(lon, lat)` enqueues a fully-formed
        // `Op::Command(UserCommand::Map(MapAction::Jump(LonLat)))` on
        // the shared op buffer; the App drains and dispatches.
        let (lua, _handles, ops) = install_for_test();

        // Lua-side call: longitude first, then latitude.
        lua.load("ttymap.map:jump(139.7595, 35.6828)")
            .exec()
            .expect("exec");

        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Command(UserCommand::Map(MapAction::Jump(ll))) => {
                assert!((ll.lon - 139.7595).abs() < 1e-9);
                assert!((ll.lat - 35.6828).abs() < 1e-9);
            }
            other => panic!("expected Op::Command(Map(Jump)), got {other:?}"),
        }
    }

    #[test]
    fn host_map_zoom_setter_pushes_appmsg_set_zoom() {
        // `ttymap.map:zoom(level)` is fire-and-forget on the Lua side —
        // the level lands on the op buffer as
        // `Op::Command(UserCommand::Map(MapAction::SetZoom(level)))`.
        let (lua, _handles, ops) = install_for_test();
        lua.load("ttymap.map:zoom(7.5)").exec().expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Command(UserCommand::Map(MapAction::SetZoom(z))) => {
                assert!((z - 7.5).abs() < 1e-9)
            }
            other => panic!("expected Op::Command(Map(SetZoom)), got {other:?}"),
        }
    }

    #[test]
    fn host_map_zoom_getter_reads_shared_cell() {
        // `ttymap.map:zoom()` (no args) reads the host-mirrored zoom
        // cell. The host writes via the same Arc the userdata holds,
        // so simulate a dispatch refresh by writing to `handles.zoom`
        // directly and assert Lua sees the new value. Symmetric with
        // the `:center()` pattern. Confirms (a) no-arg call doesn't
        // accidentally fall through to the setter and (b) the value
        // round-trips as a Lua number.
        let (lua, handles, ops) = install_for_test();
        *handles.zoom.lock().unwrap() = 9.25;
        let z: f64 = lua.load("return ttymap.map:zoom()").eval().expect("eval");
        assert!((z - 9.25).abs() < 1e-9);
        // Calling the getter must not enqueue a setter request.
        assert!(ops.borrow().is_empty());
    }

    #[test]
    fn host_map_fly_to_pushes_appmsg_fly_to() {
        // `ttymap.map:fly_to(lon, lat, zoom)` packs both into a single
        // `Op::Command(UserCommand::Map(MapAction::FlyTo))` so the host
        // emits one dispatch per call (single redraw, no intermediate
        // frame).
        let (lua, _handles, ops) = install_for_test();
        lua.load("ttymap.map:fly_to(139.7595, 35.6828, 12.0)")
            .exec()
            .expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Command(UserCommand::Map(MapAction::FlyTo {
                center,
                zoom,
            })) => {
                assert!((center.lon - 139.7595).abs() < 1e-9);
                assert!((center.lat - 35.6828).abs() < 1e-9);
                assert!((zoom - 12.0).abs() < 1e-9);
            }
            other => panic!("expected Op::Command(Map(FlyTo)), got {other:?}"),
        }
    }

    #[test]
    fn api_frame_to_ansi_returns_nil_until_a_frame_arrives() {
        // No frame has been mirrored into the shared cell yet, so
        // the read side returns nil. Plugins gate their export on
        // this and surface "no frame yet" via `ttymap.notify`.
        let (lua, _handles, _ops) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.api.frame.to_ansi()"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn api_frame_to_ansi_returns_string_after_frame_set() {
        // After a [`MapFrame`] is written into the shared cell —
        // which `App::handle_event` does on every `FrameReady` —
        // `to_ansi()` returns the rendered string.
        use ttymap_engine::geo::LonLat;
        use ttymap_engine::map::render::frame::MapFrame;

        let lua = mlua::Lua::new();
        let shared = LuaHostShared::empty();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let _handles = install(
            &lua,
            shared.clone(),
            crate::compositor::op::new_ops_buffer(),
            bus,
            crate::lua::new_lua_registry(),
        )
        .expect("install ttymap table");

        // Empty frame still renders to something deterministic
        // (an empty string per `MapFrame::to_ansi` semantics).
        {
            let mut slot = shared.current_frame.lock().unwrap();
            *slot = Some(MapFrame {
                cells: Vec::new(),
                cols: 0,
                rows: 0,
                center: LonLat { lon: 0.0, lat: 0.0 },
                zoom: 1.0,
            });
        }

        let s: String = lua
            .load(r#"return ttymap.api.frame.to_ansi()"#)
            .eval()
            .expect("eval");
        assert_eq!(s, "");
    }

    #[test]
    fn api_frame_on_tick_subscribes_each_callback_against_the_bus() {
        // Each `ttymap.api.frame.on_tick(fn)` call subscribes
        // directly to the bus (one entry per call).
        let lua = mlua::Lua::new();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let _handles = install(
            &lua,
            LuaHostShared::empty(),
            crate::compositor::op::new_ops_buffer(),
            bus.clone(),
            crate::lua::new_lua_registry(),
        )
        .expect("install ttymap table");
        lua.load(
            r#"
            ttymap.api.frame.on_tick(function() end)
            ttymap.api.frame.on_tick(function() end)
            "#,
        )
        .exec()
        .expect("exec");
        assert_eq!(
            bus.count("tick"),
            2,
            "two on_tick calls -> two bus subscribers"
        );
    }

    #[test]
    fn on_event_subscribes_against_the_named_bucket() {
        // `ttymap.on_event(name, fn)` — generic surface. Each call
        // subscribes directly under its event name on the shared
        // bus; multiple distinct names land in distinct buckets.
        let lua = mlua::Lua::new();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let _handles = install(
            &lua,
            LuaHostShared::empty(),
            crate::compositor::op::new_ops_buffer(),
            bus.clone(),
            crate::lua::new_lua_registry(),
        )
        .expect("install ttymap table");
        lua.load(
            r#"
            ttymap.on_event("tick", function() end)
            ttymap.on_event("frame_ready", function() end)
            ttymap.on_event("frame_ready", function() end)
            "#,
        )
        .exec()
        .expect("exec");
        assert_eq!(bus.count("tick"), 1);
        assert_eq!(bus.count("frame_ready"), 2);
    }

    #[test]
    fn on_event_returns_handle_whose_remove_drops_subscriber() {
        // The handle returned to Lua exposes a single `:remove()`
        // method; calling it must remove that exact subscriber from
        // the bus. Idempotent: a second call is a no-op.
        let lua = mlua::Lua::new();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let _handles = install(
            &lua,
            LuaHostShared::empty(),
            crate::compositor::op::new_ops_buffer(),
            bus.clone(),
            crate::lua::new_lua_registry(),
        )
        .expect("install ttymap table");
        lua.load(
            r#"
            handle = ttymap.on_event("frame_ready", function() end)
            "#,
        )
        .exec()
        .expect("subscribe");
        assert_eq!(bus.count("frame_ready"), 1);
        lua.load(r#"handle:remove(); handle:remove()"#)
            .exec()
            .expect("remove");
        assert_eq!(
            bus.count("frame_ready"),
            0,
            "handle:remove() must drop the subscriber"
        );
    }

    #[test]
    fn on_event_rejects_empty_name() {
        // Empty event names would land in a HashMap bucket that's
        // unreachable from any sensible dispatch call — surface an
        // error at register time so the plugin author finds it.
        let lua = mlua::Lua::new();
        let bus = std::rc::Rc::new(crate::event::EventBus::default());
        let _handles = install(
            &lua,
            LuaHostShared::empty(),
            crate::compositor::op::new_ops_buffer(),
            bus,
            crate::lua::new_lua_registry(),
        )
        .expect("install ttymap table");
        let result: mlua::Result<()> = lua.load(r#"ttymap.on_event("", function() end)"#).exec();
        assert!(result.is_err(), "empty event name should error");
    }

    #[test]
    fn url_encode_round_trips_query_chars() {
        let (lua, _handles, _ops) = install_for_test();
        // Spaces become `+`, reserved chars become `%HH`, unicode is
        // percent-encoded byte by byte.
        let encoded: String = lua
            .load(r#"return ttymap.http:url_encode("São Paulo?")"#)
            .eval()
            .expect("eval");
        assert_eq!(encoded, "S%C3%A3o+Paulo%3F");
        let plain: String = lua
            .load(r#"return ttymap.http:url_encode("abc-_.~")"#)
            .eval()
            .expect("eval");
        assert_eq!(plain, "abc-_.~");
    }

    #[test]
    fn parse_json_round_trips_primitives() {
        let (lua, _handles, _ops) = install_for_test();
        let n: i64 = lua
            .load(r#"return ttymap.json:parse("42")"#)
            .eval()
            .expect("eval");
        assert_eq!(n, 42);
        let s: String = lua
            .load(r#"return ttymap.json:parse('"hi"')"#)
            .eval()
            .expect("eval");
        assert_eq!(s, "hi");
        let b: bool = lua
            .load(r#"return ttymap.json:parse("true")"#)
            .eval()
            .expect("eval");
        assert!(b);
    }

    #[test]
    fn parse_json_object_becomes_string_keyed_table() {
        let (lua, _handles, _ops) = install_for_test();
        let (name, age): (String, i64) = lua
            .load(
                r#"
                local t = ttymap.json:parse('{"name": "alice", "age": 30}')
                return t.name, t.age
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(name, "alice");
        assert_eq!(age, 30);
    }

    #[test]
    fn parse_json_array_is_one_indexed_in_lua() {
        let (lua, _handles, _ops) = install_for_test();
        // Lua arrays are 1-indexed; t[1] is the first element.
        let (first, third, len): (i64, i64, i64) = lua
            .load(
                r#"
                local t = ttymap.json:parse("[10, 20, 30]")
                return t[1], t[3], #t
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(first, 10);
        assert_eq!(third, 30);
        assert_eq!(len, 3);
    }

    #[test]
    fn parse_json_invalid_returns_nil() {
        let (lua, _handles, _ops) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("not json !")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn parse_json_null_is_nil() {
        let (lua, _handles, _ops) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("null")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn notify_lua_api_enqueues_publish_op_with_typed_level() {
        // `ttymap.notify(msg, opts)` enqueues `Op::Publish(Event::Notify
        // {...})` onto the shared `OpsBuffer`. `App::apply_ops` later
        // hands it to `EventBus::publish`. Default level is `info`;
        // explicit `level = "warn" / "error"` parses through.
        let (lua, _handles, ops) = install_for_test();

        lua.load(
            r#"
            ttymap.notify("ok")
            ttymap.notify("watch out", { level = "warn" })
            ttymap.notify("boom", { level = "error" })
            "#,
        )
        .exec()
        .expect("notify lua exec");

        let drained: Vec<Op> = std::mem::take(&mut *ops.borrow_mut());
        let notifies: Vec<(String, Level)> = drained
            .into_iter()
            .filter_map(|op| match op {
                Op::Publish(Event::Notify { message, level }) => Some((message, level)),
                _ => None,
            })
            .collect();
        assert_eq!(notifies.len(), 3, "three notify calls -> three publishes");
        assert_eq!(notifies[0], ("ok".to_string(), Level::Info));
        assert_eq!(notifies[1], ("watch out".to_string(), Level::Warn));
        assert_eq!(notifies[2], ("boom".to_string(), Level::Error));
    }

    #[test]
    fn log_namespace_methods_round_trip() {
        // `ttymap.log:info/warn/error` are thin wrappers — no return
        // value, no error path. The unit test confirms the bindings
        // exist and accept a string. Anything observable downstream
        // (target = "lua[<plugin>]") is exercised by integration; here
        // we just want a panic-free round trip.
        let (lua, _handles, _ops) = install_for_test();
        lua.load(
            r#"
            ttymap.log:info("info-ok")
            ttymap.log:warn("warn-ok")
            ttymap.log:error("error-ok")
            "#,
        )
        .exec()
        .expect("log methods must round-trip");
    }

    #[test]
    fn sgp4_namespace_propagates_iss_through_lua() {
        // End-to-end: a Lua script calls parse_tle + propagate and
        // gets a position table back. Catches bridge wiring bugs
        // (userdata borrow, namespace install, table return shape)
        // that the standalone sgp4 module tests miss.
        let (lua, _handles, _ops) = install_for_test();
        let pos: mlua::Table = lua
            .load(
                r#"
                local tle = ttymap.sgp4:parse_tle(
                    "ISS (ZARYA)\n" ..
                    "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927\n" ..
                    "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537"
                )
                return ttymap.sgp4:propagate(tle, 1220568000)
                "#,
            )
            .eval()
            .expect("propagate from Lua");
        let lon: f64 = pos.get("lon").expect("lon");
        let lat: f64 = pos.get("lat").expect("lat");
        let alt: f64 = pos.get("alt_km").expect("alt_km");
        let vel: f64 = pos.get("vel_kms").expect("vel_kms");
        assert!((-180.0..=180.0).contains(&lon));
        assert!((-90.0..=90.0).contains(&lat));
        assert!(
            (300.0..500.0).contains(&alt),
            "altitude {alt} km not LEO-ish",
        );
        assert!((7.0..8.0).contains(&vel), "velocity {vel} not ISS-ish");
    }

    #[test]
    fn api_card_open_pushes_component_and_returns_handle() {
        // `ttymap.api.card.open(spec)` must do two things on the same
        // call: enqueue an `Op::Push` onto the shared `OpsBuffer` so
        // the App can push the component onto the compositor stack,
        // and hand back a `CardHandle` whose `:close()` enqueues
        // `Op::Close` keyed by the same id. Both behaviours are
        // independent of any `App` plumbing — this is the unit-level
        // proof that the primitive itself is wired right.
        let (lua, _handles, ops) = install_for_test();
        lua.load(
            r#"
            local h = ttymap.api.card.open({
                name = "demo",
                layout = { anchor = "left", width = 30 },
                render = function() return { "hello" } end,
            })
            ttymap_test_handle = h
            "#,
        )
        .exec()
        .expect("exec");
        // Exactly one Op::Push must be enqueued — `card.open` pushes
        // per call, no implicit dedup.
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one card.open -> one Op");
        let push_id = match &drained[0] {
            crate::compositor::op::Op::Push { id, .. } => *id,
            other => panic!("expected Op::Push, got {:?}", other),
        };
        // Close the handle from Lua — must enqueue Op::Close keyed by
        // the same id reserved at the call site.
        lua.load("ttymap_test_handle:close()")
            .exec()
            .expect("close");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one close() -> one Op");
        match &drained[0] {
            crate::compositor::op::Op::Close(id) => assert_eq!(*id, push_id),
            other => panic!("expected Op::Close, got {:?}", other),
        }
    }

    #[test]
    fn api_palette_open_pushes_component_and_returns_handle() {
        // Mirror of `api_card_open_pushes_component_and_returns_handle`:
        // `ttymap.api.palette.open(spec)` must enqueue an `Op::Push`
        // for the wrapped `PaletteComponent` and hand back a
        // `PaletteHandle` whose `:close()` enqueues `Op::Close` keyed
        // by the same id — no `App` plumbing required.
        let (lua, _handles, ops) = install_for_test();
        lua.load(
            r#"
            local h = ttymap.api.palette.open({
                prompt = "/",
                filter = function(_) end,
                items = function() return {} end,
                execute = function(_) return { close = true } end,
                is_loading = function() return false end,
            })
            ttymap_test_palette = h
            "#,
        )
        .exec()
        .expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one palette.open -> one Op");
        let push_id = match &drained[0] {
            crate::compositor::op::Op::Push { id, .. } => *id,
            other => panic!("expected Op::Push, got {:?}", other),
        };
        // `:close()` is idempotent: each call enqueues an Op::Close —
        // close_by_id treats the second one as a no-op once the
        // component is already off the stack.
        lua.load("ttymap_test_palette:close(); ttymap_test_palette:close()")
            .exec()
            .expect("close");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 2, "two close() -> two Op::Close");
        for op in drained {
            match op {
                crate::compositor::op::Op::Close(id) => assert_eq!(id, push_id),
                other => panic!("expected Op::Close, got {:?}", other),
            }
        }
    }
}
