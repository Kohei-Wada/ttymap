//! Lua → Frontend operation vocabulary.
//!
//! [`Op`] is the typed output of a Lua subsystem tick / callback: each
//! variant describes a single same-thread "do this on the App" request
//! that the Lua bridge enqueues into a shared [`OpsBuffer`] and that
//! Frontend drains once per iteration.
//!
//! Replaces the older mix of three same-thread mechanisms — the
//! per-component `CloseFlag` (Arc<AtomicBool> polled every frame),
//! the per-plugin `push_rx` queue (Box<dyn Component>), and (still)
//! [`crate::lua::sender::LuaSender`] — with a single typed buffer.
//! Close + push are migrated; the LuaSender → `Op::Intent` migration
//! is the next PR.

use std::cell::RefCell;
use std::rc::Rc;

use crate::frontend::compositor::{CardId, Component};

/// A same-thread request from the Lua subsystem to the App.
///
/// Lua callbacks (handle `:close()`, `api.card.open`, etc.) push these
/// into a shared [`OpsBuffer`]; Frontend drains the buffer once per
/// loop iteration and applies each op.
///
/// `Op::Push` and `Op::Close` are wired today. The remaining
/// `LuaSender → Op::Intent` migration (the next PR) makes this the
/// single Lua → Frontend transport.
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
}

impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Push { id, .. } => f.debug_struct("Push").field("id", id).finish(),
            Self::Close(id) => f.debug_tuple("Close").field(id).finish(),
        }
    }
}

/// Shared, single-threaded buffer that accumulates [`Op`]s from Lua
/// callbacks and is drained by Frontend per iteration.
///
/// `Rc<RefCell<...>>`: same-thread sharing across the API closures
/// (held inside the Lua VM via captured clones), the returned handles
/// (`CardHandle` / `PaletteHandle`), and the runtime [`LuaHandle`]
/// (read by Frontend). The Lua VM is single-threaded by mlua design,
/// and the buffer never crosses threads.
pub type OpsBuffer = Rc<RefCell<Vec<Op>>>;

/// Construct an empty [`OpsBuffer`]. Called once at composition root
/// and cloned into every site that needs to enqueue ops.
pub fn new_ops_buffer() -> OpsBuffer {
    Rc::new(RefCell::new(Vec::new()))
}
