//! Lua → Frontend operation vocabulary.
//!
//! [`Op`] is the typed output of a Lua subsystem tick / callback: each
//! variant describes a single same-thread "do this on the App" request
//! that the Lua bridge enqueues into a shared [`OpsBuffer`] and that
//! Frontend drains once per iteration.
//!
//! Replaces the older mix of three same-thread mechanisms — the
//! per-component `CloseFlag` (Arc<AtomicBool> polled every frame), the
//! per-plugin `push_rx` queue (Box<dyn Component>), and the chatty
//! [`crate::lua::sender::LuaSender`] — with a single typed buffer.
//! This PR migrates only the close path; the others follow.

use std::cell::RefCell;
use std::rc::Rc;

use crate::frontend::compositor::CardId;

/// A same-thread request from the Lua subsystem to the App.
///
/// Lua callbacks (handle `:close()`, `api.card.open`, etc.) push these
/// into a shared [`OpsBuffer`]; Frontend drains the buffer once per
/// loop iteration and applies each op.
///
/// Currently only [`Op::Close`] is in use. The other variants are
/// reserved for the upcoming `push_tx` and `LuaSender` migrations so
/// the buffer becomes the single Lua → Frontend transport.
#[derive(Debug)]
pub enum Op {
    /// Pop the component matching `id` off the compositor stack.
    /// Emitted by `CardHandle::close` / `PaletteHandle::close`. Silent
    /// no-op when `id` is not on the stack (handle closed twice, or
    /// the component already self-closed via `win.close()`).
    Close(CardId),
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
