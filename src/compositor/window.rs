//! [`Window`] — capability-constrained handle passed to components.
//!
//! Components receive a `&mut Window` on every hook (`handle_event`
//! and `poll`). They express intent by calling methods on it:
//!
//! ```ignore
//! fn handle_event(&mut self, ev: KeyEvent, win: &mut Window) {
//!     if ev.code == KeyCode::Esc {
//!         win.close();
//!     } else if enter_with_selection {
//!         win.emit(AppMsg::Jump(loc));
//!         win.close();
//!     } else if ev.code == KeyCode::Char('/') {
//!         win.close();
//!         win.open(Box::new(SearchComponent::new()));
//!     }
//! }
//! ```
//!
//! Method calls queue into [`WindowOps`]. The compositor drains the
//! queue after the hook returns and applies the ops atomically in a
//! deterministic order: `close` → `opens` (with TypeId dedup) → and
//! the collected `msgs` are returned to `App::dispatch`.
//!
//! # Why a handle instead of a return value
//!
//! Returning an `EventResult` enum with one variant per op
//! combination (`Close`, `Push`, `CloseAndPush`, …) does not scale
//! — every new compound op needs a new variant. The handle queues
//! primitive ops so compounds are expressed by composition. Plugin
//! still cannot hold `&mut Compositor` or mutate the stack directly;
//! the compositor is the sole applier of the queue, so invariants
//! (focus, dedup, clamp) remain framework-enforced.

use crate::app::AppMsg;
use crate::compositor::{Component, Context};

/// Queue of actions a [`Component`] hook recorded through [`Window`].
/// Drained and applied by the compositor after the hook returns.
#[derive(Default)]
pub(crate) struct WindowOps {
    /// `true` if the plugin called [`Window::close`]. Pops the
    /// calling component. Applied before `opens` so `close + open`
    /// replaces the component in the stack slot.
    pub close: bool,
    /// Components queued by [`Window::open`]. Each goes through
    /// TypeId dedup when pushed; a duplicate of an existing stack
    /// entry shifts focus to the existing one instead.
    pub opens: Vec<Box<dyn Component>>,
    /// Messages for [`App::dispatch`](crate::app::App). Returned
    /// from `Compositor::handle_event` to the caller (App), which
    /// dispatches them after the ops have been applied.
    pub msgs: Vec<AppMsg>,
    /// `true` if the plugin called [`Window::ignore`]. Meaningful
    /// only when no other op was queued — in that case the
    /// compositor re-delivers the event to the base layer (unless
    /// the handler already was the base). Ignored otherwise.
    pub ignored: bool,
}

impl WindowOps {
    /// `true` iff the hook made no state-changing call. When the
    /// hook's only effect was `ignore()`, this is also true (ignore
    /// itself is a signal, not an op).
    pub(crate) fn is_ignorable_noop(&self) -> bool {
        !self.close && self.opens.is_empty() && self.msgs.is_empty()
    }
}

/// Handle components receive on every hook. Constrained by design:
/// no `&mut Compositor` is reachable through it, so components
/// cannot break focus / stack invariants even if buggy.
///
/// Read-only accessors (`ctx`) give the component what it needs for
/// decision-making without granting any capability.
pub struct Window<'a> {
    ops: &'a mut WindowOps,
    ctx: &'a Context,
}

impl<'a> Window<'a> {
    pub(crate) fn new(ops: &'a mut WindowOps, ctx: &'a Context) -> Self {
        Self { ops, ctx }
    }

    /// App-level snapshot passed for this hook (map center, theme id).
    pub fn ctx(&self) -> &Context {
        self.ctx
    }

    /// Pop the calling component from the stack after the hook
    /// returns. Idempotent — a second call is a no-op. Applied
    /// before `open()`s so `close(); open(c);` replaces this
    /// component with `c`.
    pub fn close(&mut self) {
        self.ops.close = true;
    }

    /// Push `c` on top of the stack after the hook returns. Subject
    /// to TypeId dedup: if a component of the same concrete type is
    /// already on the stack, focus shifts to the existing instance
    /// and `c` is dropped.
    pub fn open(&mut self, c: Box<dyn Component>) {
        self.ops.opens.push(c);
    }

    /// Queue `msg` for `App::dispatch`. Dispatched by the caller
    /// (App) after the compositor has applied `close` / `open`. For
    /// typical `emit + close` patterns this means the msg still
    /// fires, but the component is already popped when it runs —
    /// identical to the old `EventResult::Close(msgs)` semantic.
    pub fn emit(&mut self, msg: AppMsg) {
        self.ops.msgs.push(msg);
    }

    /// Signal "this event isn't mine". With no other op queued,
    /// the compositor falls through to the base layer (if this
    /// component isn't already it). If combined with `close` /
    /// `open` / `emit`, the flag is silently dropped — the
    /// component clearly handled the event.
    pub fn ignore(&mut self) {
        self.ops.ignored = true;
    }
}
