//! `ttymap.api.frame.on_tick(fn) -> EventHandle` and
//! `ttymap.on_event(name, fn) -> EventHandle` — Lua-facing handle to
//! one bus subscription, whose only method is `:remove()`.
//!
//! Disposable shape (VS Code-style): the Rust side returns a handle
//! at registration time; calling `:remove()` removes that exact
//! subscriber from the [`EventBus`]. Idempotent — a second `:remove()`
//! is a no-op (the bus's `remove` returns false but doesn't panic).
//!
//! Distinct type from [`super::card_handle::CardHandle`] /
//! [`super::palette_handle::PaletteHandle`] because the receptacle
//! and verb differ — the latter close compositor stack entries
//! (`Op::Close`), this one removes a bus subscription. Same pattern,
//! intentionally different identity so `:remove()` vs `:close()`
//! reads naturally to plugin authors.

use std::rc::Rc;

use mlua::UserData;

use crate::event::EventBus;

/// Lua-facing handle returned by `ttymap.api.frame.on_tick(...)` and
/// `ttymap.on_event(name, fn)`. Holds a back-reference to the bus
/// plus the `(event_name, id)` pair the bus needs to drop the right
/// subscriber.
pub struct EventHandle {
    bus: Rc<EventBus>,
    event_name: &'static str,
    id: u64,
}

impl EventHandle {
    pub fn new(bus: Rc<EventBus>, event_name: &'static str, id: u64) -> Self {
        Self {
            bus,
            event_name,
            id,
        }
    }
}

impl UserData for EventHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            this.bus.remove(this.event_name, this.id);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;

    #[test]
    fn remove_drops_the_subscriber_and_is_idempotent() {
        let bus = Rc::new(EventBus::default());
        let lua = mlua::Lua::new();
        lua.load(
            r#"
            fired = 0
            function bump() fired = fired + 1 end
            "#,
        )
        .exec()
        .unwrap();
        let f: mlua::Function = lua.globals().get("bump").unwrap();
        let key = lua.create_registry_value(f).unwrap();
        let id = bus.subscribe_lua("frame_ready", "test", lua.clone(), key);

        bus.publish(Event::FrameReady);
        let n: i64 = lua.globals().get("fired").unwrap();
        assert_eq!(n, 1);

        let ud = lua
            .create_userdata(EventHandle::new(Rc::clone(&bus), "frame_ready", id))
            .unwrap();
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .unwrap();

        bus.publish(Event::FrameReady);
        let n: i64 = lua.globals().get("fired").unwrap();
        assert_eq!(n, 1, "subscriber removed; second publish must not refire");
    }
}
