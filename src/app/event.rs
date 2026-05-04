//! Unified event vocabulary for the App's main loop.
//!
//! `AppEvent` is the single payload carried on the App-level `mpsc`
//! channel that every off-main-thread (or deferred main-thread)
//! source pushes into. Four variants:
//!
//! - [`AppEvent::Command`] — wraps a [`UserCommand`]: every
//!   fire-and-forget intent emitter (keymap / mouse when dispatch
//!   can't run inline; Lua plugins use the shared [`crate::core::compositor::op::OpsBuffer`])
//!   goes through this. Synchronous emitters (compositor.poll,
//!   palette.execute) still call [`super::App::dispatch`]
//!   directly without the channel.
//! - [`AppEvent::FrameReady`] — the render thread hands back a
//!   completed [`MapFrame`] for the App to display next paint cycle.
//! - [`AppEvent::Input`] — a raw terminal event (keyboard / mouse /
//!   resize / paste) read by the input thread. The main loop matches
//!   on it and dispatches the appropriate downstream `UserCommand`s.
//! - [`AppEvent::Wake`] — periodic wake-up from the frame timer
//!   thread. Replaces the old `recv_timeout` polling: the main loop
//!   now blocks on `recv()` and the timer guarantees per-iteration
//!   work (animation plugins, overlay redraws) still ticks at a
//!   predictable cadence even with no input or render activity.
//!   *Not* the same thing as the Lua-side `"tick"` bus event:
//!   `Wake` is "main loop, please run an iteration", whereas the
//!   Lua `"tick"` event fires from inside the per-frame draw closure
//!   against a live `MapApi`. They're aligned in cadence (one per
//!   draw) but live at different layers.
//!
//! The split (rather than adding `FrameReady` directly to [`UserCommand`])
//! is intentional: `UserCommand` derives `PartialEq` for keymap binding
//! lookups, and `MapFrame` is a per-frame grid buffer whose equality
//! is meaningless and expensive. Wrapping keeps `UserCommand`'s vocabulary
//! tight and the keymap path unchanged, while the unified queue still
//! gets one drain instead of N polled sources.

use crate::core::map::render::frame::MapFrame;

use crate::UserCommand;

/// Unified event payload drained from the App-level `mpsc` channel.
///
/// See module-level docs for the rationale behind the variant split.
#[derive(Debug)]
pub enum AppEvent {
    /// A [`UserCommand`] emitted off-thread or deferred from inside a
    /// Lua callback. Dispatched through [`super::App::dispatch`] in
    /// arrival order.
    Command(UserCommand),
    /// A freshly rendered [`MapFrame`] from the render thread. Stored
    /// on the App as the next paint snapshot; multiple in flight =>
    /// last one wins, matching the prior dedicated-channel behaviour.
    FrameReady(MapFrame),
    /// Raw terminal event (key / mouse / resize / paste) read off
    /// the input thread. The main loop classifies and dispatches it
    /// — same downstream code path as the previous inline
    /// `crossterm::event::poll` block, just sourced through the
    /// unified queue.
    Input(crossterm::event::Event),
    /// Periodic wake-up from the frame timer thread. The main loop's
    /// only response is to fall through into the per-iteration
    /// draw + overlay-redraw rate-check, so per-frame work still
    /// ticks at the configured cadence even when no other event is
    /// arriving. Distinct from the Lua-side `"tick"` event, which
    /// fires from inside the draw closure against a `MapApi`.
    Wake,
}

impl From<UserCommand> for AppEvent {
    fn from(msg: UserCommand) -> Self {
        AppEvent::Command(msg)
    }
}
