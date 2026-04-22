//! App-level message vocabulary consumed by [`App::dispatch`](super::App::dispatch).
//!
//! `AppMsg` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async pending output, the mouse adapter, and
//! (one day) external control surfaces like an HTTP/JSON-RPC front.
//! Everyone speaks the same vocabulary.
//!
//! In GoF Command-pattern terms this is the **Command** role expressed
//! as a closed algebraic type: each variant is an imperative intent
//! (`Map`, `Jump`, `CycleFocus`) that the Receiver ([`super::App`])
//! executes in [`App::dispatch`](super::App::dispatch). Invokers
//! (keymap, palette, plugins) **return `Vec<AppMsg>`** and never
//! execute anything themselves — the dispatcher is the sole
//! side-effect boundary. Note the naming split: "command" is reserved
//! for user-facing concepts (the CLI subcommand in `crate::commands`
//! and the `:`-palette entries), while internal intent is `AppMsg`.
//!
//! Surface activation (palette open, plugin activate) intentionally
//! does *not* live here — those are focus transitions, handled
//! internally by the
//! [`Compositor`](crate::compositor::Compositor) via
//! [`Window::open`](crate::compositor::window::Window::open) /
//! [`Window::close`](crate::compositor::window::Window::close) calls
//! from a [`Component`](crate::compositor::Component). Keeping them
//! off `AppMsg` means the focus state machine isn't coupled to the
//! dispatch table.

use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::map::Action;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; interpreted
/// by [`App::dispatch`](super::App::dispatch) inside the event loop.
///
/// Map-level intents are nested under [`AppMsg::Map`] because
/// [`MapState`](crate::map::MapState) owns its own command vocabulary
/// ([`Action`]) and consumes it through a single entry
/// ([`MapState::process_action`](crate::map::MapState::process_action)).
/// Other variants sit at the top level: each is handled directly by
/// an `App::dispatch` arm and there is no intermediate sub-system to
/// delegate to.
#[derive(Debug, Clone, PartialEq)]
pub enum AppMsg {
    /// Dispatch a map-state action (pan, zoom, reset, quit, ...).
    Map(Action),
    /// Jump the map to a specific location — produced by search /
    /// here-plugin / any future picker that yields a `LonLat`.
    Jump(LonLat),
    /// Switch the running theme. Cross-cutting: rebuilds the styler
    /// (on the render thread) and the UI colour cache.
    SetTheme(ThemeId),
    /// Mouse cursor moved to the given terminal cell. Emitted by the
    /// mouse adapter on every event so the overlay cursor readout
    /// goes through dispatch like every other user-intent state
    /// change.
    CursorMoved(u16, u16),
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Terminal resized — update the map viewport and the render
    /// thread's canvas dimensions. Arguments are the new terminal
    /// size in cells.
    Resize(u16, u16),
}
