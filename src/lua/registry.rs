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

use mlua::{Lua, RegistryKey};

use crate::compositor::MapApi;
use crate::lua::ttymap::map_api;

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
    /// Carved out as its own method (rather than a generic
    /// `dispatch(name, args)`) because `MapApi` borrows the ratatui
    /// buffer for one frame and the Lua-facing handle has to be
    /// constructed inside `Lua::scope` — a shape that doesn't fit a
    /// fully-generic dispatch signature without HRTB gymnastics.
    /// When future event surfaces fire from elsewhere, they get
    /// their own `dispatch_*` method shaped to the data they pass.
    ///
    /// Errors are logged + swallowed per callback so a single broken
    /// plugin must not freeze the loop.
    pub fn dispatch_tick(&self, map: &mut MapApi<'_>) {
        let Some(subs) = self.subscribers.get("tick") else {
            return;
        };
        let cell = std::cell::RefCell::new(map);
        for sub in subs {
            let result: mlua::Result<()> = sub.lua.scope(|scope| {
                let map_table = map_api::make_map_table(&sub.lua, scope, &cell)?;
                let f: mlua::Function = sub.lua.registry_value(&sub.callback)?;
                f.call::<()>(map_table)
            });
            if let Err(e) = result {
                log::warn!("lua[{}]: tick subscriber failed: {}", sub.name, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LonLat;
    use crate::map::render::frame::MapFrame;
    use crate::map::render::overlay::UserPolyline;
    use crate::theme::{DARK, UiTheme};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

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
}
