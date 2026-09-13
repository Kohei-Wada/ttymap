//! Coverage for the `ttymap` global that [`super::install`] builds:
//! namespace wiring, and the op-buffer shape each Lua call lowers to.

use super::*;
use ttymap_engine::map::MapAction;
use ttymap_shared::UserCommand;

/// Every `ttymap.<name>` userdata namespace [`super::install`] wires up.
///
/// The install itself is driven by a local `(name, userdata)` table;
/// this mirrors that list so drift in either direction fails
/// [`ttymap_table_is_installed_with_namespaces`] rather than silently
/// dropping a namespace from the plugin surface.
const USERDATA_NAMESPACES: &[&str] = &[
    "http", "map", "json", "sgp4", "tile", "config", "help", "log", "storage",
];

/// Helper for tests: install the `ttymap` table into a fresh Lua
/// and hand back the host handles + the shared op buffer. Mirrors
/// the production install path; the bus is dropped since these
/// tests don't exercise registration or dispatch.
fn install_for_test() -> (
    mlua::Lua,
    LuaHostHandles,
    ttymap_tui::compositor::op::OpsBuffer,
) {
    let lua = mlua::Lua::new();
    let ops = ttymap_tui::compositor::op::new_ops_buffer();
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let registry = crate::new_lua_registry();
    // Resolve real XDG dirs so `ttymap.storage` wires up (it
    // gates on `AppDirs.data` and silently skips when None).
    // Matches pre-#362 behavior where every test relied on a
    // real `$HOME` being present.
    let dirs = ttymap_config::AppDirs::resolve();
    let handles = install(
        &lua,
        LuaHostShared::empty(),
        ops.clone(),
        bus,
        ticks,
        registry,
        dirs.as_ref(),
    )
    .expect("install ttymap table");
    (lua, handles, ops)
}

#[test]
fn ttymap_table_is_installed_with_namespaces() {
    let (lua, _handles, _ops) = install_for_test();
    let ttymap: Table = lua.globals().get("ttymap").expect("ttymap global");

    // Every userdata-valued key on `ttymap` is a namespace — the
    // other install sites (register / imperative / notify) only
    // ever set functions, tables or strings.
    let mut installed: Vec<String> = ttymap
        .pairs::<String, mlua::Value>()
        .map(|pair| pair.expect("ttymap pair"))
        .filter(|(_, value)| matches!(value, mlua::Value::UserData(_)))
        .map(|(name, _)| name)
        .collect();
    installed.sort();

    let mut expected: Vec<String> = USERDATA_NAMESPACES.iter().map(|s| s.to_string()).collect();
    expected.sort();

    assert_eq!(installed, expected);
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

    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        ttymap_tui::compositor::op::Op::Command(UserCommand::Map(MapAction::Jump(ll))) => {
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
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        ttymap_tui::compositor::op::Op::Command(UserCommand::Map(MapAction::SetZoom(z))) => {
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
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1);
    match &drained[0] {
        ttymap_tui::compositor::op::Op::Command(UserCommand::Map(MapAction::FlyTo {
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
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let _handles = install(
        &lua,
        shared.clone(),
        ttymap_tui::compositor::op::new_ops_buffer(),
        bus,
        ticks,
        crate::new_lua_registry(),
        None,
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
fn api_frame_on_tick_subscribes_each_callback_against_the_tick_registry() {
    // Each `ttymap.api.frame.on_tick(fn)` call subscribes
    // directly to the per-frame `TickRegistry` (not the
    // typed-event bus — the tick payload is a borrowed
    // `MapApi` that can't ride `&Event`).
    let lua = mlua::Lua::new();
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let _handles = install(
        &lua,
        LuaHostShared::empty(),
        ttymap_tui::compositor::op::new_ops_buffer(),
        bus,
        ticks.clone(),
        crate::new_lua_registry(),
        None,
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
        ticks.len(),
        2,
        "two on_tick calls -> two tick-registry subscribers",
    );
}

#[test]
fn on_event_subscribes_against_the_named_bucket() {
    // `ttymap.on_event(name, fn)` — generic surface. Routing:
    // `"tick"` goes to the per-frame `TickRegistry`, everything
    // else subscribes against the typed-event bus.
    let lua = mlua::Lua::new();
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let _handles = install(
        &lua,
        LuaHostShared::empty(),
        ttymap_tui::compositor::op::new_ops_buffer(),
        bus.clone(),
        ticks.clone(),
        crate::new_lua_registry(),
        None,
    )
    .expect("install ttymap table");
    lua.load(
        r#"
        ttymap.on_event("tick", function() end)
        ttymap.on_event("notify", function() end)
        ttymap.on_event("notify", function() end)
        "#,
    )
    .exec()
    .expect("exec");
    assert_eq!(ticks.len(), 1, "tick lands in the tick registry");
    assert_eq!(
        bus.count("tick"),
        0,
        "tick never lands on the typed-event bus",
    );
    assert_eq!(bus.count("notify"), 2);
}

#[test]
fn on_event_returns_handle_whose_remove_drops_subscriber() {
    // The handle returned to Lua exposes a single `:remove()`
    // method; calling it must remove that exact subscriber from
    // the bus. Idempotent: a second call is a no-op.
    let lua = mlua::Lua::new();
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let _handles = install(
        &lua,
        LuaHostShared::empty(),
        ttymap_tui::compositor::op::new_ops_buffer(),
        bus.clone(),
        ticks,
        crate::new_lua_registry(),
        None,
    )
    .expect("install ttymap table");
    lua.load(
        r#"
        handle = ttymap.on_event("notify", function() end)
        "#,
    )
    .exec()
    .expect("subscribe");
    assert_eq!(bus.count("notify"), 1);
    lua.load(r#"handle:remove(); handle:remove()"#)
        .exec()
        .expect("remove");
    assert_eq!(
        bus.count("notify"),
        0,
        "handle:remove() must drop the subscriber",
    );
}

#[test]
fn on_event_rejects_empty_name() {
    // Empty event names would land in a HashMap bucket that's
    // unreachable from any sensible dispatch call — surface an
    // error at register time so the plugin author finds it.
    let lua = mlua::Lua::new();
    let bus = std::rc::Rc::new(ttymap_shared::event::EventBus::default());
    let ticks = std::rc::Rc::new(crate::tick::TickRegistry::default());
    let _handles = install(
        &lua,
        LuaHostShared::empty(),
        ttymap_tui::compositor::op::new_ops_buffer(),
        bus,
        ticks,
        crate::new_lua_registry(),
        None,
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
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1, "one card.open -> one Op");
    let push_id = match &drained[0] {
        ttymap_tui::compositor::op::Op::Push { id, .. } => *id,
        other => panic!("expected Op::Push, got {:?}", other),
    };
    // Close the handle from Lua — must enqueue Op::Close keyed by
    // the same id reserved at the call site.
    lua.load("ttymap_test_handle:close()")
        .exec()
        .expect("close");
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1, "one close() -> one Op");
    match &drained[0] {
        ttymap_tui::compositor::op::Op::Close(id) => assert_eq!(*id, push_id),
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
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 1, "one palette.open -> one Op");
    let push_id = match &drained[0] {
        ttymap_tui::compositor::op::Op::Push { id, .. } => *id,
        other => panic!("expected Op::Push, got {:?}", other),
    };
    // `:close()` is idempotent: each call enqueues an Op::Close —
    // close_by_id treats the second one as a no-op once the
    // component is already off the stack.
    lua.load("ttymap_test_palette:close(); ttymap_test_palette:close()")
        .exec()
        .expect("close");
    let drained: Vec<ttymap_tui::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
    assert_eq!(drained.len(), 2, "two close() -> two Op::Close");
    for op in drained {
        match op {
            ttymap_tui::compositor::op::Op::Close(id) => assert_eq!(id, push_id),
            other => panic!("expected Op::Close, got {:?}", other),
        }
    }
}
