//! [`EventBus`] — main-thread pub/sub registry.
//!
//! Subscribers (plain `Fn(&Event)` closures) register against an
//! event name, and [`Self::publish`] fans an [`Event`] out to every
//! registered subscriber for that name.
//!
//! # Lua-agnostic
//!
//! The bus stores `Rc<dyn Fn(&Event)>` — it does not know about
//! `mlua`. Lua plugins subscribe through `ttymap.on_event(name, fn)`
//! (see `lua/api/register.rs`), which wraps the Lua callback in a
//! Rust closure that captures the `mlua::Lua` + `RegistryKey` and
//! handles the Lua call site (error logging, typed arg conversion).
//! The per-frame `tick` event is **not** routed through the bus at
//! all — it needs a borrowed `MapApi` that can't fit `&Event`, and
//! lives in `lua/tick.rs` with its own `TickRegistry`.
//!
//! # Thread model
//!
//! Dispatch is **main-thread only** (`mlua::Lua` is `!Send`, and Lua
//! callbacks captured into subscriber closures must run there). The
//! bus uses [`RefCell`] (not `Mutex`) since there is exactly one
//! accessor.
//!
//! Cross-thread publish is reachable through the App-level mpsc:
//! producers wrap an [`Event`] in
//! [`crate::app::AppEvent::Bus`](crate::app::AppEvent::Bus) and
//! `send` it; the main loop drains and calls [`Self::publish`].
//!
//! # Identity and removal
//!
//! Each successful `subscribe` call returns a monotonic `u64`. The
//! Lua surface (e.g. `EventHandle`) wraps that ID so plugins can
//! `:remove()` themselves later without name/lhs collisions.
//!
//! Dispatch is implemented as **ID snapshot + per-call lookup**: the
//! bucket's IDs are cloned up front, then each subscriber is cloned
//! out under a short-lived borrow that drops before the callback
//! runs. A callback may therefore call [`Self::remove`] (which takes
//! `borrow_mut`) — concurrent removal naturally drops out at the
//! next iteration's lookup. Re-subscription during dispatch is also
//! safe; the new entry is *not* visible to the current dispatch (its
//! ID is not in the snapshot) and will fire from the next `publish`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::Event;

/// One subscriber slot — its monotonic id paired with the callback.
type Subscriber = (u64, Rc<dyn Fn(&Event)>);

/// Pub/sub registry. Keyed by event name (the same string Lua
/// scripts pass to `ttymap.on_event`). Subscribers within one bucket
/// fire in registration order.
#[derive(Default)]
pub struct EventBus {
    subscribers: RefCell<HashMap<&'static str, Vec<Subscriber>>>,
    next_id: AtomicU64,
}

impl EventBus {
    fn allocate_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a subscriber for `event_name`. Returns a monotonic
    /// ID that can be passed to [`Self::remove`]. The closure is
    /// stored as `Rc<dyn Fn>` so dispatch can clone it out and drop
    /// the bus borrow before invoking — letting the callback call
    /// `subscribe` / `remove` on the bus without panicking.
    pub fn subscribe<F: Fn(&Event) + 'static>(&self, event_name: &'static str, f: F) -> u64 {
        let id = self.allocate_id();
        self.subscribers
            .borrow_mut()
            .entry(event_name)
            .or_default()
            .push((id, Rc::new(f)));
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

    /// Total subscriber count across every bucket.
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
    /// `event.name()`.
    ///
    /// Dispatch is snapshot-driven: a callback may call
    /// [`Self::remove`] (including on itself) or `subscribe` from
    /// inside without disturbing the in-flight dispatch.
    pub fn publish(&self, event: Event) {
        let name = event.name();
        let ids: Vec<u64> = self
            .subscribers
            .borrow()
            .get(name)
            .map(|bucket| bucket.iter().map(|(id, _)| *id).collect())
            .unwrap_or_default();

        for id in ids {
            let extracted = {
                let subs = self.subscribers.borrow();
                let Some(bucket) = subs.get(name) else {
                    continue;
                };
                bucket
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, f)| Rc::clone(f))
            };
            if let Some(f) = extracted {
                f(&event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Level;
    use std::cell::Cell;

    fn notify(msg: &str) -> Event {
        Event::Notify {
            message: msg.to_string(),
            level: Level::Info,
        }
    }

    #[test]
    fn publish_only_runs_subscribers_for_the_named_event() {
        let bus = EventBus::default();
        let other_count = Rc::new(Cell::new(0));
        let sink = other_count.clone();
        bus.subscribe("other_name", move |_| sink.set(sink.get() + 1));

        bus.publish(notify("hi"));

        assert_eq!(
            other_count.get(),
            0,
            "publish must skip buckets for other event names",
        );
    }

    #[test]
    fn publish_passes_typed_event_through() {
        let bus = EventBus::default();
        let captured: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let sink = captured.clone();
        bus.subscribe("notify", move |e| {
            let Event::Notify { message, .. } = e;
            *sink.borrow_mut() = Some(message.clone());
        });
        bus.publish(notify("hello world"));

        let msg = captured
            .borrow()
            .clone()
            .expect("subscriber must have captured");
        assert_eq!(msg, "hello world");
    }

    #[test]
    fn subscribe_returns_distinct_monotonic_ids() {
        let bus = EventBus::default();
        let id_a = bus.subscribe("notify", |_| {});
        let id_b = bus.subscribe("notify", |_| {});
        let id_c = bus.subscribe("other_name", |_| {});
        assert_ne!(id_a, id_b);
        assert!(id_b > id_a);
        assert!(id_c > id_b, "ids are global, not per-bucket");
    }

    #[test]
    fn remove_drops_only_the_named_subscriber() {
        let a = Rc::new(Cell::new(0));
        let b = Rc::new(Cell::new(0));
        let bus = EventBus::default();
        let a_sink = a.clone();
        let b_sink = b.clone();
        let id_a = bus.subscribe("notify", move |_| a_sink.set(a_sink.get() + 1));
        let _id_b = bus.subscribe("notify", move |_| b_sink.set(b_sink.get() + 1));

        bus.publish(notify("first"));
        assert_eq!(a.get(), 1);
        assert_eq!(b.get(), 1);

        assert!(bus.remove("notify", id_a));
        bus.publish(notify("second"));
        assert_eq!(a.get(), 1, "a removed, must not fire again");
        assert_eq!(b.get(), 2);

        assert!(!bus.remove("notify", id_a), "second remove returns false",);
        assert!(
            !bus.remove("nonexistent", 999),
            "missing bucket returns false",
        );
    }

    #[test]
    fn callback_can_remove_itself_during_dispatch() {
        // The bus must not panic when a callback calls
        // `EventBus::remove(...)` on itself from inside dispatch —
        // exercises the "drop borrow before calling" property the
        // Lua `:remove()` surface relies on.
        let bus = Rc::new(EventBus::default());
        let fire_count = Rc::new(Cell::new(0));

        // Subscriber A: counts its own fires AND removes itself.
        let bus_for_a = Rc::clone(&bus);
        let fire_for_a = fire_count.clone();
        let id_holder: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let id_holder_for_a = id_holder.clone();
        let id = bus.subscribe("notify", move |_| {
            fire_for_a.set(fire_for_a.get() + 1);
            bus_for_a.remove("notify", id_holder_for_a.get());
        });
        id_holder.set(id);

        bus.publish(notify("a"));
        bus.publish(notify("b"));

        assert_eq!(
            fire_count.get(),
            1,
            "callback should fire once then be removed",
        );
    }

    #[test]
    fn re_subscribe_during_dispatch_does_not_panic() {
        // Subscribing during dispatch must not deadlock the
        // RefCell. The new entry is *not* visible to the in-flight
        // publish (it's not in the snapshot), but a subsequent
        // publish must fire it.
        let bus = Rc::new(EventBus::default());
        let inner_fired = Rc::new(Cell::new(0));
        let inner_fired_clone = inner_fired.clone();
        let bus_for_closure = Rc::clone(&bus);
        let outer_fired = Rc::new(Cell::new(0));
        let outer_fired_clone = outer_fired.clone();
        bus.subscribe("notify", move |_| {
            outer_fired_clone.set(outer_fired_clone.get() + 1);
            let inner_sink = inner_fired_clone.clone();
            bus_for_closure.subscribe("notify", move |_| {
                inner_sink.set(inner_sink.get() + 1);
            });
        });

        bus.publish(notify("a"));
        assert_eq!(
            inner_fired.get(),
            0,
            "inner registered during dispatch must not be in this snapshot",
        );
        assert_eq!(outer_fired.get(), 1);

        bus.publish(notify("b"));
        assert!(inner_fired.get() >= 1);
    }
}
