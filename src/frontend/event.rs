//! Unified event vocabulary for the App's main loop.
//!
//! `AppEvent` is the single payload carried on the App-level `mpsc`
//! channel that every off-main-thread (or deferred main-thread)
//! source pushes into. Four variants:
//!
//! - [`AppEvent::Intent`] — wraps an [`AppMsg`]: every fire-and-forget
//!   user-intent emitter (Lua plugins via the host channel,
//!   keymap/mouse when the dispatch can't run inline) goes through
//!   this. Synchronous emitters (compositor.poll, palette.execute)
//!   still call [`super::App::dispatch`] directly without the channel.
//! - [`AppEvent::FrameReady`] — the render thread hands back a
//!   completed [`MapFrame`] for the App to display next paint cycle.
//! - [`AppEvent::Input`] — a raw terminal event (keyboard / mouse /
//!   resize / paste) read by the input thread. The main loop matches
//!   on it and dispatches the appropriate downstream `AppMsg`s.
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
//! The split (rather than adding `FrameReady` directly to [`AppMsg`])
//! is intentional: `AppMsg` derives `PartialEq` for keymap binding
//! lookups, and `MapFrame` is a per-frame grid buffer whose equality
//! is meaningless and expensive. Wrapping keeps `AppMsg`'s vocabulary
//! tight and the keymap path unchanged, while the unified queue still
//! gets one drain instead of N polled sources.

use crate::map::render::frame::MapFrame;

use super::AppMsg;

/// Unified event payload drained from the App-level `mpsc` channel.
///
/// See module-level docs for the rationale behind the variant split.
#[derive(Debug)]
pub enum AppEvent {
    /// A user-intent [`AppMsg`] emitted off-thread or deferred from
    /// inside a Lua callback. Dispatched through
    /// [`super::App::dispatch`] in arrival order.
    Intent(AppMsg),
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

impl From<AppMsg> for AppEvent {
    fn from(msg: AppMsg) -> Self {
        AppEvent::Intent(msg)
    }
}
