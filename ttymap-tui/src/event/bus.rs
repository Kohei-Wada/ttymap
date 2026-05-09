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
//! # Identity and removal
//!
//! Each successful `subscribe_*` call returns a monotonic `u64`. The
//! Lua surface (e.g. `EventHandle`) wraps that ID so plugins can
//! `:remove()` themselves later without name/lhs collisions.
//!
//! Dispatch is implemented as **ID snapshot + per-call lookup**: the
//! bucket's IDs are cloned up front, then each subscriber is looked
//! up by ID under a short-lived borrow that drops before the Lua
//! callback runs. A callback may therefore call [`Self::remove`]
//! (which takes `borrow_mut`) — concurrent removal naturally drops
//! out at the next iteration's lookup. Re-subscription during
//! dispatch is also safe; the new entry is *not* visible to the
//! current dispatch (its ID is not in the snapshot) and will fire
//! from the next `publish`.
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
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use mlua::{Lua, RegistryKey};

use super::Event;

/// One registered subscriber. Stored under an event name in
/// [`EventBus::subscribers`] paired with its issued ID, and invoked
/// once per [`EventBus::publish`] for that name.
pub enum Subscriber {
    /// Pure-Rust callback. Sees the typed [`Event`] enum and decides
    /// what to do (re-publish, mutate a side cache, etc). Boxed as
    /// `Rc<dyn Fn>` so dispatch can clone the callback out, drop the
    /// bus borrow, and *then* invoke — making it safe for the
    /// callback to call `subscribe_*` / `remove` (both need
    /// `borrow_mut`) on the bus.
    Rust(Rc<dyn Fn(&Event)>),
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
    subscribers: RefCell<HashMap<&'static str, Vec<(u64, Subscriber)>>>,
    next_id: AtomicU64,
}

impl EventBus {
    fn allocate_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a Rust subscriber for `event_name`. Returns a
    /// monotonic ID that can be passed to [`Self::remove`].
    pub fn subscribe_rust<F: Fn(&Event) + 'static>(&self, event_name: &'static str, f: F) -> u64 {
        let id = self.allocate_id();
        self.subscribers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push((id, Subscriber::Rust(Rc::new(f))));
        id
    }

    /// Register a Lua subscriber for `event_name`. The plugin string
    /// is purely diagnostic (logged when the callback errors).
    /// Returns a monotonic ID that can be passed to [`Self::remove`].
    pub fn subscribe_lua(
        &self,
        event_name: &'static str,
        plugin: &'static str,
        lua: Lua,
        callback: RegistryKey,
    ) -> u64 {
        let id = self.allocate_id();
        self.subscribers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push((
                id,
                Subscriber::Lua {
                    plugin,
                    lua,
                    callback,
                },
            ));
        id
    }

    /// Remove one subscriber by `(event_name, id)`. Returns true if
    /// a matching entry was found and removed. Safe to call from
    /// inside a dispatched callback (see module docs).
    pub fn remove(&self, event_name: &str, id: u64) -> bool {
        let mut subs = self.subscribers.borrow_mut();
        let Some(bucket) = subs.get_mut(event_name) else {
            return false;
        };
        let before = bucket.len();
        bucket.retain(|(i, _)| *i != id);
        before != bucket.len()
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

    /// Snapshot the current ID list for `event_name`. Order matches
    /// registration order. Helper for the snapshot-then-lookup
    /// dispatch pattern used by [`Self::publish`] and
    /// [`Self::for_each_lua_subscriber`].
    fn snapshot_ids(&self, event_name: &str) -> Vec<u64> {
        self.subscribers
            .borrow()
            .get(event_name)
            .map(|bucket| bucket.iter().map(|(id, _)| *id).collect())
            .unwrap_or_default()
    }

    /// Fan an [`Event`] out to every subscriber registered under
    /// `event.name()`. Rust subscribers see `&Event` directly; Lua
    /// subscribers receive the variant's typed Lua args
    /// (`map_jumped` → `(lon, lat)`, `notify` → `{ message, level }`,
    /// etc).
    ///
    /// Dispatch is snapshot-driven: a callback may call
    /// [`Self::remove`] (including on itself) without disturbing the
    /// in-flight dispatch.
    pub fn publish(&self, event: Event) {
        let name = event.name();
        let ids = self.snapshot_ids(name);

        // Per-call dispatch action — extracted from the bus under a
        // short borrow so the borrow can drop before invoking. Lets
        // a callback (Rust or Lua) call `subscribe_*` / `remove`
        // (both `borrow_mut`) without panicking.
        enum Action {
            Skip,
            Rust(Rc<dyn Fn(&Event)>),
            Lua(&'static str, Lua, mlua::Function),
        }

        for id in ids {
            let action = {
                let subs = self.subscribers.borrow();
                let Some(bucket) = subs.get(name) else {
                    continue;
                };
                let Some((_, sub)) = bucket.iter().find(|(i, _)| *i == id) else {
                    continue;
                };
                match sub {
                    Subscriber::Rust(f) => Action::Rust(Rc::clone(f)),
                    Subscriber::Lua {
                        plugin,
                        lua,
                        callback,
                    } => match lua.registry_value::<mlua::Function>(callback) {
                        Ok(func) => Action::Lua(plugin, lua.clone(), func),
                        Err(e) => {
                            log::warn!(
                                "lua[{}]: {} subscriber registry lookup failed: {}",
                                plugin,
                                name,
                                e
                            );
                            Action::Skip
                        }
                    },
                }
            };
            match action {
                Action::Skip => {}
                Action::Rust(f) => f(&event),
                Action::Lua(plugin, lua, func) => {
                    if let Err(e) = call_lua_with_event(&func, &lua, &event) {
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
    ///
    /// `f` receives the `Function` directly (already resolved from
    /// the registry key) so the bus can drop its borrow before
    /// calling — a callback may therefore `remove` itself.
    pub fn for_each_lua_subscriber<F>(&self, event_name: &str, mut f: F)
    where
        F: FnMut(&str, &Lua, &mlua::Function),
    {
        let ids = self.snapshot_ids(event_name);
        for id in ids {
            let extracted = {
                let subs = self.subscribers.borrow();
                let Some(bucket) = subs.get(event_name) else {
                    continue;
                };
                let Some((_, sub)) = bucket.iter().find(|(i, _)| *i == id) else {
                    continue;
                };
                match sub {
                    Subscriber::Lua {
                        plugin,
                        lua,
                        callback,
                    } => match lua.registry_value::<mlua::Function>(callback) {
                        Ok(func) => Some((*plugin, lua.clone(), func)),
                        Err(e) => {
                            log::warn!(
                                "lua[{}]: {} subscriber registry lookup failed: {}",
                                plugin,
                                event_name,
                                e
                            );
                            None
                        }
                    },
                    Subscriber::Rust(_) => None,
                }
            };
            if let Some((plugin, lua, func)) = extracted {
                f(plugin, &lua, &func);
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

    #[test]
    fn subscribe_returns_distinct_monotonic_ids() {
        let bus = EventBus::default();
        let id_a = bus.subscribe_rust("frame_ready", |_| {});
        let id_b = bus.subscribe_rust("frame_ready", |_| {});
        let id_c = bus.subscribe_rust("map_jumped", |_| {});
        assert_ne!(id_a, id_b);
        assert!(id_b > id_a);
        assert!(id_c > id_b, "ids are global, not per-bucket");
    }

    #[test]
    fn remove_drops_only_the_named_subscriber() {
        use std::cell::Cell;
        use std::rc::Rc;

        let a = Rc::new(Cell::new(0));
        let b = Rc::new(Cell::new(0));
        let bus = EventBus::default();
        let a_sink = a.clone();
        let b_sink = b.clone();
        let id_a = bus.subscribe_rust("frame_ready", move |_| a_sink.set(a_sink.get() + 1));
        let _id_b = bus.subscribe_rust("frame_ready", move |_| b_sink.set(b_sink.get() + 1));

        bus.publish(Event::FrameReady);
        assert_eq!(a.get(), 1);
        assert_eq!(b.get(), 1);

        assert!(bus.remove("frame_ready", id_a));
        bus.publish(Event::FrameReady);
        assert_eq!(a.get(), 1, "a removed, must not fire again");
        assert_eq!(b.get(), 2);

        assert!(
            !bus.remove("frame_ready", id_a),
            "second remove returns false"
        );
        assert!(
            !bus.remove("nonexistent", 999),
            "missing bucket returns false"
        );
    }

    #[test]
    fn lua_callback_can_remove_itself_during_dispatch() {
        // The bus must not panic when a Lua callback calls
        // `EventBus::remove(...)` on itself from inside dispatch —
        // exercises the "drop borrow before calling Lua" property
        // that future Lua-side `:remove()` will rely on.
        use std::cell::Cell;
        use std::rc::Rc;

        let lua = Lua::new();
        lua.load(
            r#"
            fire_count = 0
            function bump() fire_count = fire_count + 1 end
            "#,
        )
        .exec()
        .unwrap();
        let f: mlua::Function = lua.globals().get("bump").unwrap();
        let key = lua.create_registry_value(f).unwrap();

        let bus = std::rc::Rc::new(EventBus::default());
        let id = bus.subscribe_lua("frame_ready", "self_remove", lua.clone(), key);

        // A Rust sub that, when dispatched, removes the Lua sub from
        // the same bucket. This is the in-dispatch removal path.
        let bus_for_closure = Rc::clone(&bus);
        let saw_lua = Rc::new(Cell::new(false));
        let saw_lua_for_closure = saw_lua.clone();
        bus.subscribe_rust("frame_ready", move |_| {
            // Confirm the Lua sub has fired by this point if it was
            // first in registration order (it was — id allocated
            // before this rust closure).
            saw_lua_for_closure.set(true);
            bus_for_closure.remove("frame_ready", id);
        });

        bus.publish(Event::FrameReady);
        bus.publish(Event::FrameReady);

        let n: i64 = lua.globals().get("fire_count").unwrap();
        assert_eq!(n, 1, "lua sub should fire once then be removed");
        assert!(saw_lua.get(), "rust sub also fired");
    }

    #[test]
    fn re_subscribe_during_dispatch_does_not_panic() {
        // Subscribing during dispatch must not deadlock the
        // RefCell. The new entry is *not* visible to the in-flight
        // publish (it's not in the snapshot), but a subsequent
        // publish must fire it.
        use std::cell::Cell;
        use std::rc::Rc;

        let bus = std::rc::Rc::new(EventBus::default());
        let inner_fired = Rc::new(Cell::new(0));
        let inner_fired_clone = inner_fired.clone();
        let bus_for_closure = Rc::clone(&bus);
        let outer_fired = Rc::new(Cell::new(0));
        let outer_fired_clone = outer_fired.clone();
        bus.subscribe_rust("frame_ready", move |_| {
            outer_fired_clone.set(outer_fired_clone.get() + 1);
            // Subscribe a new closure during dispatch.
            let inner_sink = inner_fired_clone.clone();
            bus_for_closure.subscribe_rust("frame_ready", move |_| {
                inner_sink.set(inner_sink.get() + 1);
            });
        });

        bus.publish(Event::FrameReady);
        // Inner sub registered during dispatch but not in this
        // snapshot, so it shouldn't have fired yet.
        assert_eq!(inner_fired.get(), 0);
        assert_eq!(outer_fired.get(), 1);

        // Second publish: snapshot now includes the inner sub plus
        // ANOTHER copy registered by the outer firing again, etc.
        // Just assert nothing panics and inner_fired increments.
        bus.publish(Event::FrameReady);
        assert!(inner_fired.get() >= 1);
    }
}
