//! Component → host effect vocabulary.
//!
//! [`Op`] is the typed output of a component hook or Lua callback:
//! each variant describes a single same-thread "do this on my behalf"
//! request that the producer enqueues and the host (`Compositor` for
//! Push/Close, `App::dispatch` for Intent) applies after the producer
//! returns.
//!
//! Two producers ride this enum:
//! - **Component hooks** (`handle_key` / `poll`) enqueue via the
//!   [`Window`](super::window::Window) handle into a stack-local
//!   `WindowOps`; the compositor drains it the moment the hook returns.
//! - **Lua callbacks** (handle `:close()`, `api.card.open`,
//!   `ttymap.map:jump`, …) enqueue into the shared [`OpsBuffer`];
//!   `App::apply_lua_ops` drains it once per loop iteration.
//!
//! Both paths converge in `App::apply_ops`, which dispatches by
//! variant. Single typed buffer carrying [`Op::Push`] / [`Op::Close`]
//! / [`Op::Command`].

use std::cell::RefCell;
use std::rc::Rc;

use crate::compositor::{CardId, Component};
use ttymap_core::UserCommand;
use ttymap_core::event::Event;

/// A same-thread request from a component (Rust or Lua-backed) to
/// the host. Component hooks emit these via the
/// [`Window`](super::window::Window) handle; Lua callbacks emit via
/// the shared [`OpsBuffer`]. The host (`Compositor` for stack ops,
/// `App::dispatch` for commands) applies them after the producer
/// returns.
pub enum Op {
    /// Push a component onto the compositor stack with a
    /// caller-supplied [`CardId`]. The id is reserved at the
    /// `api.card.open` / `api.palette.open` call site so the
    /// returned handle can target this exact component for close.
    Push {
        id: CardId,
        component: Box<dyn Component>,
    },
    /// Pop the component matching `id` off the compositor stack.
    /// Emitted by `CardHandle::close` / `PaletteHandle::close`. Silent
    /// no-op when `id` is not on the stack (handle closed twice, or
    /// the component already self-closed via `win.close()`).
    Close(CardId),
    /// Dispatch a [`UserCommand`] through `App::dispatch`.
    /// Emitted by Lua-facing host methods (`ttymap.map:jump` /
    /// `:zoom` / `:fly_to`, `ttymap.api.frame.export`, …) — the
    /// canonical command vocabulary every other emitter (keymap,
    /// mouse, palette) already speaks.
    Command(UserCommand),
    /// Publish an [`Event`] onto the bus for fan-out to subscribers.
    /// Emitted by Lua-facing producers like `ttymap.notify(msg)` —
    /// the bus is owned by [`crate::lua::LuaHandle`], so reaching it
    /// from inside a Lua callback would require sharing an `Rc<EventBus>`
    /// across the bridge. Riding the existing buffer keeps Lua's
    /// dependency on the host minimal: it just enqueues a typed
    /// effect, and `App::apply_ops` calls `bus.publish` on the
    /// main-thread side.
    Publish(Event),
}

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Push { id, .. } => f.debug_struct("Push").field("id", id).finish(),
            Self::Close(id) => f.debug_tuple("Close").field(id).finish(),
            Self::Command(c) => f.debug_tuple("Command").field(c).finish(),
            Self::Publish(e) => f.debug_tuple("Publish").field(&e.name()).finish(),
        }
    }
}

/// Shared, single-threaded buffer that accumulates [`Op`]s from Lua
/// callbacks and is drained by App per iteration. (Component hooks
/// use a separate, stack-local `WindowOps` instead — see
/// [`Window`](super::window::Window).)
///
/// `Rc<RefCell<...>>`: same-thread sharing across the API closures
/// (held inside the Lua VM via captured clones), the returned handles
/// (`CardHandle` / `PaletteHandle`), and the runtime [`LuaHandle`]
/// (read by App). The Lua VM is single-threaded by mlua design,
/// and the buffer never crosses threads.
pub type OpsBuffer = Rc<RefCell<Vec<Op>>>;

/// Construct an empty [`OpsBuffer`]. Called once at composition root
/// and cloned into every site that needs to enqueue ops.
pub fn new_ops_buffer() -> OpsBuffer {
    Rc::new(RefCell::new(Vec::new()))
}
