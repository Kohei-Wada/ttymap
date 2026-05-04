//! Lua-side event bus.
//!
//! Each plugin script subscribes to host events via either:
//!
//! - `ttymap.on_event(name, fn)` — generic subscription
//! - `ttymap.api.frame.on_tick(fn)` — sugar for `on_event("tick", fn)`
//!
//! Both lower into a [`Subscriber`] held in [`LuaEventBus`] under the
//! event name key. The bus is a textbook pub/sub registry on the Lua
//! side: each event name owns a `Vec<Subscriber>`, and dispatch finds
//! the bucket for the event name and runs every callback in
//! registration order.
//!
//! Today there is one event surface — `"tick"` — fired from
//! [`crate::ui::draw`] once per frame against the live `MapApi`.
//! Future events (`"frame_ready"`, `"map_jumped"`, …) will land in
//! the same bus and follow the same pattern: pass an event-specific
//! Lua argument to each subscriber, log + swallow errors per call so
//! one buggy plugin can't freeze the host.

use std::collections::HashMap;

use mlua::{IntoLuaMulti, Lua, RegistryKey};

use crate::lua::MapApi;
use crate::lua::api::map_table;

/// Canonical Lua-facing event names. Centralised so the host emit
/// site and any internal subscriber agree on the spelling — Lua
/// scripts use bare strings (`ttymap.on_event("frame_ready", fn)`)
/// since they live outside the Rust crate, but everything inside
/// goes through these constants.
pub mod names {
    /// Per-frame draw hook. Subscribers fire from inside `ui::draw`
    /// against a live `MapApi`. Distinct from `AppEvent::Wake`,
    /// which is the main-loop wake-up signal.
    pub const TICK: &str = "tick";
    /// Render thread produced a fresh `MapFrame`. No payload — the
    /// frame is heavy + the live snapshot is read via `ttymap.map`
    /// accessors.
    pub const FRAME_READY: &str = "frame_ready";
    /// Map state recentred via `MapAction::Jump`. Payload: `(lon, lat)`.
    pub const MAP_JUMPED: &str = "map_jumped";
    /// Direct zoom set via `MapAction::SetZoom`. Payload: `zoom: f64`.
    pub const MAP_ZOOM_SET: &str = "map_zoom_set";
    /// Composite recentre+zoom via `MapAction::FlyTo`. Payload:
    /// `(lon, lat, zoom)`.
    pub const MAP_FLEW_TO: &str = "map_flew_to";
    /// Theme switched. Payload: `theme name string`.
    pub const THEME_CHANGED: &str = "theme_changed";
    /// Terminal resized. Payload: `(cols, rows)`.
    pub const RESIZED: &str = "resized";
    /// `MapAction::ExportFrame` ran (regardless of success). No payload.
    pub const FRAME_EXPORTED: &str = "frame_exported";
}

/// One registered subscriber. The Lua state is the plugin's setup
/// state (cloned from `register_one`); the registry key points at the
/// callback the script handed to `ttymap.on_event` /
/// `ttymap.api.frame.on_tick`.
pub struct Subscriber {
    pub name: &'static str,
    pub lua: Lua,
    pub callback: RegistryKey,
}

/// Pub/sub registry for Lua-side event subscriptions.
///
/// Keyed by event name (currently `"tick"`; new event types add new
/// keys). Each bucket is a `Vec<Subscriber>`, dispatched in
/// registration order. Adding a new event surface is two lines: pick
/// a name and call [`Self::dispatch_tick`]-style from wherever the
/// event fires.
#[derive(Default)]
pub struct LuaEventBus {
    /// Per-event subscriber list. `&'static str` keys because event
    /// names are baked at the host (script-supplied names from
    /// `ttymap.on_event` get leaked once at register time, mirroring
    /// the plugin-name/source treatment in `register_one`).
    subscribers: HashMap<&'static str, Vec<Subscriber>>,
}

impl LuaEventBus {
    /// Add a callback to the bucket for `event_name`. Order within
    /// a bucket is registration order (`Vec::push`).
    pub fn subscribe(&mut self, event_name: &'static str, sub: Subscriber) {
        self.subscribers.entry(event_name).or_default().push(sub);
    }

    /// Total number of subscribers across every event. Used by the
    /// `every_bundled_script_registers` smoke test as a lower bound
    /// on chrome plugins that subscribe to `"tick"`.
    pub fn len(&self) -> usize {
        self.subscribers.values().map(|v| v.len()).sum()
    }

    /// Number of subscribers for a specific event name.
    pub fn count(&self, event_name: &str) -> usize {
        self.subscribers
            .get(event_name)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.subscribers.values().all(|v| v.is_empty())
    }

    /// Fire the per-frame `"tick"` bucket against a live `MapApi`.
    ///
    /// Carved out as its own method (rather than just calling
    /// [`Self::dispatch`]) because `MapApi` borrows the ratatui
    /// buffer for one frame and the Lua-facing handle has to be
    /// constructed inside `Lua::scope` — a shape that doesn't fit
    /// the simple `IntoLuaMulti` signature of `dispatch`. Other
    /// events that pass plain values use `dispatch` directly.
    ///
    /// Errors are logged + swallowed per callback so a single broken
    /// plugin must not freeze the loop.
    pub fn dispatch_tick(&self, map: &mut MapApi<'_>) {
        let Some(subs) = self.subscribers.get(names::TICK) else {
            return;
        };
        let cell = std::cell::RefCell::new(map);
        for sub in subs {
            let result: mlua::Result<()> = sub.lua.scope(|scope| {
                let map_table = map_table::make_map_table(&sub.lua, scope, &cell)?;
                let f: mlua::Function = sub.lua.registry_value(&sub.callback)?;
                f.call::<()>(map_table)
            });
            if let Err(e) = result {
                log::warn!("lua[{}]: tick subscriber failed: {}", sub.name, e);
            }
        }
    }

    /// Fire the bucket for `event_name` against every registered
    /// subscriber, passing `args` to each. Used for non-tick events
    /// whose payload is a plain Lua value (or tuple of values) — see
    /// the [`names`] module for the canonical event-name set.
    ///
    /// `Clone` is required because mlua's `Function::call` consumes
    /// the args, and the bus may have multiple subscribers per
    /// event. For payloads cheap to clone (numbers, short strings,
    /// small tuples) this is the natural shape; expensive payloads
    /// should provide their own `dispatch_*` variant the way
    /// [`Self::dispatch_tick`] does.
    ///
    /// Errors are logged + swallowed per callback so one broken
    /// plugin can't break the dispatch loop.
    pub fn dispatch<A>(&self, event_name: &str, args: A)
    where
        A: IntoLuaMulti + Clone,
    {
        let Some(subs) = self.subscribers.get(event_name) else {
            return;
        };
        for sub in subs {
            let result: mlua::Result<()> = (|| {
                let f: mlua::Function = sub.lua.registry_value(&sub.callback)?;
                f.call::<()>(args.clone())
            })();
            if let Err(e) = result {
                log::warn!("lua[{}]: {} subscriber failed: {}", sub.name, event_name, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DARK;
    use crate::theme::UiTheme;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ttymap_engine::geo::LonLat;
    use ttymap_engine::map::render::frame::MapFrame;
    use ttymap_engine::map::render::overlay::UserPolyline;

    fn fixture(area_w: u16, area_h: u16) -> (Buffer, Rect, MapFrame, UiTheme) {
        let area = Rect::new(0, 0, area_w, area_h);
        let buf = Buffer::empty(area);
        let frame = MapFrame {
            cells: Vec::new(),
            cols: area_w,
            rows: area_h,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 1.0,
        };
        let theme = UiTheme::from_palette(&DARK);
        (buf, area, frame, theme)
    }

    /// Build a fresh Lua state with a counter closure stashed in the
    /// registry. Returns the state, the registry key for the
    /// callback, and the global name where the counter lives so the
    /// test can read it back.
    fn lua_with_counter(global: &str) -> (Lua, RegistryKey) {
        let lua = Lua::new();
        // Counter starts at 0; each call bumps it by 1.
        lua.load(format!(
            r#"
            {global} = 0
            function tick_{global}(_map)
                {global} = {global} + 1
            end
            "#
        ))
        .exec()
        .expect("lua exec");
        let f: mlua::Function = lua.globals().get(format!("tick_{global}")).expect("get fn");
        let key = lua.create_registry_value(f).expect("registry");
        (lua, key)
    }

    #[test]
    fn dispatch_tick_calls_each_subscriber_once_per_call() {
        let (lua_a, key_a) = lua_with_counter("a");
        let (lua_b, key_b) = lua_with_counter("b");

        let mut bus = LuaEventBus::default();
        bus.subscribe(
            "tick",
            Subscriber {
                name: "a",
                lua: lua_a.clone(),
                callback: key_a,
            },
        );
        bus.subscribe(
            "tick",
            Subscriber {
                name: "b",
                lua: lua_b.clone(),
                callback: key_b,
            },
        );
        assert_eq!(bus.len(), 2);
        assert_eq!(bus.count("tick"), 2);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        {
            let mut sink: Vec<UserPolyline> = Vec::new();
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
            bus.dispatch_tick(&mut api);
        }
        let a: i64 = lua_a.globals().get("a").expect("read a");
        let b: i64 = lua_b.globals().get("b").expect("read b");
        assert_eq!(a, 1, "first dispatch should bump a once");
        assert_eq!(b, 1, "first dispatch should bump b once");

        {
            let mut sink: Vec<UserPolyline> = Vec::new();
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
            bus.dispatch_tick(&mut api);
        }
        let a: i64 = lua_a.globals().get("a").expect("read a");
        let b: i64 = lua_b.globals().get("b").expect("read b");
        assert_eq!(a, 2, "second dispatch should bump a again");
        assert_eq!(b, 2, "second dispatch should bump b again");
    }

    /// A subscriber that throws is logged and swallowed; subsequent
    /// subscribers in the same bucket still fire. Guards against the
    /// "one buggy plugin freezes everyone" failure mode.
    #[test]
    fn dispatch_continues_after_a_subscriber_errors() {
        let lua_bad = Lua::new();
        lua_bad
            .load(r#"function bang(_map) error("boom") end"#)
            .exec()
            .expect("lua exec");
        let bad_fn: mlua::Function = lua_bad.globals().get("bang").expect("get bang");
        let bad_key = lua_bad.create_registry_value(bad_fn).expect("registry");

        let (lua_good, good_key) = lua_with_counter("good");

        let mut bus = LuaEventBus::default();
        bus.subscribe(
            "tick",
            Subscriber {
                name: "bad",
                lua: lua_bad,
                callback: bad_key,
            },
        );
        bus.subscribe(
            "tick",
            Subscriber {
                name: "good",
                lua: lua_good.clone(),
                callback: good_key,
            },
        );

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        bus.dispatch_tick(&mut api);

        let good: i64 = lua_good.globals().get("good").expect("read good");
        assert_eq!(
            good, 1,
            "broken upstream subscriber should not stop downstream subscribers"
        );
    }

    #[test]
    fn dispatch_tick_with_empty_bus_is_a_noop() {
        let bus = LuaEventBus::default();
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        bus.dispatch_tick(&mut api);
        assert!(bus.is_empty());
    }

    #[test]
    fn dispatch_only_runs_subscribers_for_the_named_event() {
        // Subscribers under a different event name must not fire when
        // we dispatch `"tick"`. This is the core pub/sub guarantee.
        let (lua_other, key_other) = lua_with_counter("other");

        let mut bus = LuaEventBus::default();
        bus.subscribe(
            "frame_ready", // not "tick"
            Subscriber {
                name: "other",
                lua: lua_other.clone(),
                callback: key_other,
            },
        );

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        bus.dispatch_tick(&mut api);

        let other: i64 = lua_other.globals().get("other").expect("read other");
        assert_eq!(
            other, 0,
            "dispatch_tick must not fire subscribers under other event names"
        );
    }

    /// `dispatch(name, args)` is the generic broadcast for non-tick
    /// events. Verifies the args round-trip into the Lua callback —
    /// here a `(f64, f64)` payload (the `map_jumped` shape) lands as
    /// two arguments.
    #[test]
    fn dispatch_passes_args_through_to_subscriber() {
        let lua = Lua::new();
        lua.load(
            r#"
            captured_lon = 0.0
            captured_lat = 0.0
            function record(lon, lat)
                captured_lon = lon
                captured_lat = lat
            end
            "#,
        )
        .exec()
        .expect("lua exec");
        let f: mlua::Function = lua.globals().get("record").expect("get fn");
        let key = lua.create_registry_value(f).expect("registry");

        let mut bus = LuaEventBus::default();
        bus.subscribe(
            names::MAP_JUMPED,
            Subscriber {
                name: "test",
                lua: lua.clone(),
                callback: key,
            },
        );
        bus.dispatch(names::MAP_JUMPED, (139.7595_f64, 35.6828_f64));

        let lon: f64 = lua.globals().get("captured_lon").expect("read lon");
        let lat: f64 = lua.globals().get("captured_lat").expect("read lat");
        assert!((lon - 139.7595).abs() < 1e-9);
        assert!((lat - 35.6828).abs() < 1e-9);
    }

    /// `dispatch` runs every subscriber for the named event in
    /// registration order; subscribers under different names stay
    /// silent. Same pub/sub guarantee as `dispatch_tick` but for the
    /// generic path.
    #[test]
    fn dispatch_fans_out_in_registration_order_and_skips_other_buckets() {
        let lua = Lua::new();
        lua.load(
            r#"
            log = {}
            function bump(tag) table.insert(log, tag) end
            "#,
        )
        .exec()
        .expect("lua exec");
        let bump: mlua::Function = lua.globals().get("bump").expect("get bump");

        let mut bus = LuaEventBus::default();
        // Two subscribers for `frame_ready`, one for an unrelated
        // event. `dispatch("frame_ready", ...)` must hit the first
        // two and skip the third.
        for tag in ["a", "b"] {
            let key = lua.create_registry_value(bump.clone()).expect("registry");
            bus.subscribe(
                names::FRAME_READY,
                Subscriber {
                    name: tag,
                    lua: lua.clone(),
                    callback: key,
                },
            );
        }
        let key_other = lua.create_registry_value(bump).expect("registry");
        bus.subscribe(
            names::THEME_CHANGED,
            Subscriber {
                name: "other",
                lua: lua.clone(),
                callback: key_other,
            },
        );

        bus.dispatch(names::FRAME_READY, "yes");
        let log: Vec<String> = lua.globals().get("log").expect("read log");
        assert_eq!(log, vec!["yes".to_string(), "yes".to_string()]);
    }
}
