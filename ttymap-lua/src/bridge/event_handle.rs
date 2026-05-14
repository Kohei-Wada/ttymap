//! `ttymap.api.frame.on_tick(fn) -> EventHandle` and
//! `ttymap.on_event(name, fn) -> EventHandle` — Lua-facing handle to
//! one subscription, whose only method is `:remove()`.
//!
//! Disposable shape (VS Code-style): the Rust side returns a handle
//! at registration time; calling `:remove()` invokes the captured
//! remove closure, which drops the subscriber from whichever
//! registry it was stored in (the [`EventBus`] for typed events, the
//! [`TickRegistry`] for `tick`). Idempotent — a second `:remove()`
//! is a no-op because every registry's `remove` returns `false`
//! without panicking on an already-dropped ID.
//!
//! Held as `Rc<dyn Fn()>` rather than a `Box` so the same handle can
//! be cloned by Lua's userdata machinery and `:remove()` is callable
//! from any clone. Cleared to a no-op closure after the first call
//! to short-circuit follow-up borrows on a stale registry.
//!
//! Distinct type from [`super::card_handle::CardHandle`] /
//! [`super::palette_handle::PaletteHandle`] because the receptacle
//! and verb differ — the latter close compositor stack entries
//! (`Op::Close`), this one removes a subscription. Same pattern,
//! intentionally different identity so `:remove()` vs `:close()`
//! reads naturally to plugin authors.
//!
//! [`EventBus`]: ttymap_core::event::EventBus
//! [`TickRegistry`]: crate::tick::TickRegistry

use std::cell::RefCell;
use std::rc::Rc;

use mlua::UserData;

/// Lua-facing handle returned by `ttymap.api.frame.on_tick(...)` and
/// `ttymap.on_event(name, fn)`. Holds a one-shot remove closure that
/// targets the right registry (bus or tick).
pub struct EventHandle {
    remove: RefCell<Option<Rc<dyn Fn()>>>,
}

impl EventHandle {
    /// Wrap a remove closure. The closure should drop the
    /// subscriber from whichever registry the caller registered
    /// against; the handle calls it at most once.
    pub fn new(remove: Rc<dyn Fn()>) -> Self {
        Self {
            remove: RefCell::new(Some(remove)),
        }
    }
}

impl UserData for EventHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            if let Some(f) = this.remove.borrow_mut().take() {
                f();
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttymap_core::event::{Event, EventBus};

    #[test]
    fn remove_drops_the_subscriber_and_is_idempotent() {
        let bus = Rc::new(EventBus::default());
        let fired: Rc<std::cell::Cell<i64>> = Rc::new(std::cell::Cell::new(0));
        let sink = fired.clone();
        let id = bus.subscribe("frame_ready", move |_| sink.set(sink.get() + 1));

        bus.publish(Event::FrameReady);
        assert_eq!(fired.get(), 1);

        let bus_for_remove = Rc::clone(&bus);
        let handle = EventHandle::new(Rc::new(move || {
            bus_for_remove.remove("frame_ready", id);
        }));

        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(handle).unwrap();
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .unwrap();

        bus.publish(Event::FrameReady);
        assert_eq!(
            fired.get(),
            1,
            "subscriber removed; second publish must not refire",
        );
    }
}
