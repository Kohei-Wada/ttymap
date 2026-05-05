//! [`EventBus`] — main-thread pub/sub registry.
//!
//! Subscribers (Rust closures and Lua callbacks) register against an
//! event name, and [`Self::publish`] fans an [`Event`] out to every
//! registered subscriber for that name. Errors in any one Lua
//! subscriber are logged + swallowed so a single broken plugin can't
//! freeze the host.
//!
//! # Thread model
//!
//! Dispatch is **main-thread only** — `mlua::Lua` is `!Send`, so Lua
//! callbacks must run there, and the [`Subscriber::Lua`] variant
//! holds non-Send state. The bus uses [`RefCell`] (not `Mutex`)
//! since there is exactly one accessor.
//!
//! Cross-thread publish is reachable through the App-level mpsc:
//! producers wrap an [`Event`] in
//! [`crate::app::AppEvent::Bus`](crate::app::AppEvent::Bus) and
//! `send` it; the main loop drains and calls [`Self::publish`].
//!
//! # Re-entry
//!
//! Subscribers must not call `subscribe_*` during a dispatch (would
//! panic on `RefCell::borrow_mut` while we hold an immutable
//! borrow). Today no plugin does this — registration happens at
//! plugin load only.
//!
//! # The `tick` event
//!
//! `tick` is **Lua-only** and lives outside this file: per-frame
//! draw needs a borrowed `MapApi`, which doesn't fit the typed
//! [`Event`] shape, and pulling `MapApi` into `event/` would create
//! a circular dep with `lua/`. The Lua-side tick dispatcher walks
//! this bus's `tick` bucket via [`Self::for_each_lua_subscriber`].

use std::cell::RefCell;
use std::collections::HashMap;

use mlua::{Lua, RegistryKey};

use super::Event;

/// One registered subscriber. Stored under an event name in
/// [`EventBus::subscribers`] and invoked once per
/// [`EventBus::publish`] for that name.
pub enum Subscriber {
    /// Pure-Rust callback. Sees the typed [`Event`] enum and decides
    /// what to do (re-publish, mutate a side cache, etc).
    Rust(Box<dyn Fn(&Event)>),
    /// Lua plugin callback. The mlua state is the plugin's setup
    /// state (cloned from `register_one`); the registry key points
    /// at the function the script handed to `ttymap.on_event` or
    /// `ttymap.api.frame.on_tick`.
    Lua {
        plugin: &'static str,
        lua: Lua,
        callback: RegistryKey,
    },
}

/// Pub/sub registry. Keyed by event name (the same string Lua
/// scripts pass to `ttymap.on_event`). Subscribers within one bucket
/// fire in registration order.
#[derive(Default)]
pub struct EventBus {
    subscribers: RefCell<HashMap<&'static str, Vec<Subscriber>>>,
}

impl EventBus {
    /// Register a Rust subscriber for `event_name`. Order within a
    /// bucket is registration order.
    pub fn subscribe_rust<F: Fn(&Event) + 'static>(&self, event_name: &'static str, f: F) {
        self.subscribers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push(Subscriber::Rust(Box::new(f)));
    }

    /// Register a Lua subscriber for `event_name`. The plugin string
    /// is purely diagnostic (logged when the callback errors).
    pub fn subscribe_lua(
        &self,
        event_name: &'static str,
        plugin: &'static str,
        lua: Lua,
        callback: RegistryKey,
    ) {
        self.subscribers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push(Subscriber::Lua {
                plugin,
                lua,
                callback,
            });
    }

    /// Total subscriber count across every bucket. Used as a
    /// lower-bound smoke check by `every_bundled_script_registers`.
    pub fn len(&self) -> usize {
        self.subscribers.borrow().values().map(|v| v.len()).sum()
    }

    /// Number of subscribers registered for one event name.
    pub fn count(&self, event_name: &str) -> usize {
        self.subscribers
            .borrow()
            .get(event_name)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.subscribers.borrow().values().all(|v| v.is_empty())
    }

    /// Fan an [`Event`] out to every subscriber registered under
    /// `event.name()`. Rust subscribers see `&Event` directly; Lua
    /// subscribers receive the variant's typed Lua args
    /// (`map_jumped` → `(lon, lat)`, `notify` → `{ message, level }`,
    /// etc).
    ///
    /// See module docs for the re-entry constraint.
    pub fn publish(&self, event: Event) {
        let name = event.name();
        let subs = self.subscribers.borrow();
        let Some(bucket) = subs.get(name) else { return };

        for sub in bucket.iter() {
            match sub {
                Subscriber::Rust(f) => f(&event),
                Subscriber::Lua {
                    plugin,
                    lua,
                    callback,
                } => {
                    let result: mlua::Result<()> = (|| {
                        let f: mlua::Function = lua.registry_value(callback)?;
                        call_lua_with_event(&f, lua, &event)
                    })();
                    if let Err(e) = result {
                        log::warn!("lua[{}]: {} subscriber failed: {}", plugin, name, e);
                    }
                }
            }
        }
    }

    /// Run `f` once per Lua subscriber of `event_name`. Used by
    /// callers that need to construct a custom Lua call site
    /// per-subscriber — today only the `tick` dispatcher in
    /// `lua/` (which builds a per-frame `MapApi` table inside
    /// `Lua::scope`). Rust subscribers are skipped.
    ///
    /// Carved out so this module stays free of `crate::lua::*`
    /// imports — the Lua-specific tick path is in `lua/` where it
    /// belongs.
    pub fn for_each_lua_subscriber<F>(&self, event_name: &str, mut f: F)
    where
        F: FnMut(&str, &Lua, &RegistryKey),
    {
        let subs = self.subscribers.borrow();
        let Some(bucket) = subs.get(event_name) else {
            return;
        };
        for sub in bucket.iter() {
            if let Subscriber::Lua {
                plugin,
                lua,
                callback,
            } = sub
            {
                f(plugin, lua, callback);
            }
        }
    }
}

/// Build the Lua arg tuple for `event` and call `f`. Each [`Event`]
/// variant maps to the same shape Lua plugins have always seen — so
/// existing scripts (`function on_jump(lon, lat) … end`) keep working.
fn call_lua_with_event(f: &mlua::Function, lua: &Lua, event: &Event) -> mlua::Result<()> {
    use crate::event::Level;
    match event {
        Event::FrameReady => f.call::<()>(()),
        Event::MapJumped(ll) => f.call::<()>((ll.lon, ll.lat)),
        Event::MapZoomSet(z) => f.call::<()>(*z),
        Event::MapFlewTo(ll, z) => f.call::<()>((ll.lon, ll.lat, *z)),
        Event::ThemeChanged(name) => f.call::<()>(name.as_str()),
        Event::Resized(c, r) => f.call::<()>((*c, *r)),
        Event::Notify { message, level } => {
            let t = lua.create_table()?;
            t.set("message", message.as_str())?;
            t.set(
                "level",
                match level {
                    Level::Info => "info",
                    Level::Warn => "warn",
                    Level::Error => "error",
                },
            )?;
            f.call::<()>(t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttymap_engine::geo::LonLat;

    #[test]
    fn publish_only_runs_subscribers_for_the_named_event() {
        let lua = Lua::new();
        lua.load(
            r#"
            other_count = 0
            function bump() other_count = other_count + 1 end
            "#,
        )
        .exec()
        .unwrap();
        let f: mlua::Function = lua.globals().get("bump").unwrap();
        let key = lua.create_registry_value(f).unwrap();

        let bus = EventBus::default();
        bus.subscribe_lua("frame_ready", "test", lua.clone(), key);
        bus.publish(Event::MapJumped(LonLat { lon: 1.0, lat: 2.0 }));

        let n: i64 = lua.globals().get("other_count").unwrap();
        assert_eq!(n, 0, "publish must skip buckets for other event names");
    }

    #[test]
    fn publish_passes_typed_args_through() {
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
        .unwrap();
        let f: mlua::Function = lua.globals().get("record").unwrap();
        let key = lua.create_registry_value(f).unwrap();

        let bus = EventBus::default();
        bus.subscribe_lua("map_jumped", "test", lua.clone(), key);
        bus.publish(Event::MapJumped(LonLat {
            lon: 139.7595,
            lat: 35.6828,
        }));

        let lon: f64 = lua.globals().get("captured_lon").unwrap();
        let lat: f64 = lua.globals().get("captured_lat").unwrap();
        assert!((lon - 139.7595).abs() < 1e-9);
        assert!((lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn notify_event_passes_table_payload_with_message_and_level() {
        let lua = Lua::new();
        lua.load(
            r#"
            captured_msg = ""
            captured_level = ""
            function on_notify(e)
                captured_msg = e.message
                captured_level = e.level
            end
            "#,
        )
        .exec()
        .unwrap();
        let f: mlua::Function = lua.globals().get("on_notify").unwrap();
        let key = lua.create_registry_value(f).unwrap();

        let bus = EventBus::default();
        bus.subscribe_lua("notify", "test", lua.clone(), key);
        bus.publish(Event::Notify {
            message: "hello".to_string(),
            level: super::super::Level::Warn,
        });

        let msg: String = lua.globals().get("captured_msg").unwrap();
        let level: String = lua.globals().get("captured_level").unwrap();
        assert_eq!(msg, "hello");
        assert_eq!(level, "warn");
    }

    #[test]
    fn rust_subscriber_sees_typed_event_directly() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let captured = Rc::new(RefCell::new(0_f64));
        let sink = captured.clone();
        let bus = EventBus::default();
        bus.subscribe_rust("map_zoom_set", move |e| {
            if let Event::MapZoomSet(z) = e {
                *sink.borrow_mut() = *z;
            }
        });
        bus.publish(Event::MapZoomSet(7.5));
        assert!((*captured.borrow() - 7.5).abs() < 1e-9);
    }

    #[test]
    fn dispatch_continues_after_a_lua_subscriber_errors() {
        let lua_bad = Lua::new();
        lua_bad
            .load(r#"function bang() error("boom") end"#)
            .exec()
            .unwrap();
        let bad_fn: mlua::Function = lua_bad.globals().get("bang").unwrap();
        let bad_key = lua_bad.create_registry_value(bad_fn).unwrap();

        let lua_good = Lua::new();
        lua_good
            .load(
                r#"
                ok_count = 0
                function bump() ok_count = ok_count + 1 end
                "#,
            )
            .exec()
            .unwrap();
        let good_fn: mlua::Function = lua_good.globals().get("bump").unwrap();
        let good_key = lua_good.create_registry_value(good_fn).unwrap();

        let bus = EventBus::default();
        bus.subscribe_lua("frame_ready", "bad", lua_bad, bad_key);
        bus.subscribe_lua("frame_ready", "good", lua_good.clone(), good_key);

        bus.publish(Event::FrameReady);

        let ok: i64 = lua_good.globals().get("ok_count").unwrap();
        assert_eq!(ok, 1, "good subscriber must still fire after bad errors");
    }

    #[test]
    fn for_each_lua_subscriber_skips_rust_and_other_buckets() {
        let lua = Lua::new();
        let make_noop = || {
            let f: mlua::Function = lua.load(r#"return function() end"#).eval().unwrap();
            lua.create_registry_value(f).unwrap()
        };

        let bus = EventBus::default();
        bus.subscribe_lua("tick", "p", lua.clone(), make_noop());
        bus.subscribe_rust("tick", |_| {});
        bus.subscribe_lua("frame_ready", "q", lua.clone(), make_noop());

        let mut count = 0;
        bus.for_each_lua_subscriber("tick", |_, _, _| count += 1);
        assert_eq!(count, 1, "must skip the Rust sub and the other bucket");
    }
}
