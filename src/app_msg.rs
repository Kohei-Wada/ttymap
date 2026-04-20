//! App-level message vocabulary + central dispatcher.
//!
//! `AppMsg` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async `pending_command`, and (one day) external
//! control surfaces like an HTTP/JSON-RPC front. Everyone speaks the
//! same vocabulary.
//!
//! [`dispatch`] is a **thin router**: each arm maps an `AppMsg` to a
//! single method on the domain type that owns the relevant state
//! (`UiState` / `MapState`). Those methods are where multi-step
//! invariants live (focus ↔ palette ↔ widgets transitions, etc.).
//! Adding a new message = one new `AppMsg` variant + one match arm +
//! the domain method it calls.

use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::render::thread::RenderHandle;
use crate::map::{Action, MapState};
use crate::ui::UiState;
use crate::ui::action::UiAction;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; dispatched by
/// [`dispatch`] inside the input pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum AppMsg {
    /// Dispatch a map-state action (pan, zoom, reset, quit, ...).
    Map(Action),
    /// Jump the map to a specific location — produced by search /
    /// here-plugin / any future picker that yields a `LonLat`.
    Jump(LonLat),
    /// Mutate UI-level state (theme, future language / export / ...).
    Ui(UiAction),
    /// Activate a plugin by its registered tag — same semantics as
    /// pressing the plugin's activation key.
    ActivatePlugin(String),
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Open the command palette with its default provider. No-op if
    /// already open.
    OpenPalette,
    /// Terminal resized — update the map viewport and the render
    /// thread's canvas dimensions. Arguments are the new terminal
    /// size in cells.
    Resize(u16, u16),
}

/// What a key or mouse event just changed. Drives how the main loop
/// reacts: a widget-only change redraws immediately (the map frame is
/// unchanged); a map change only requests a new render — the main
/// loop will redraw when a fresh frame arrives, avoiding a
/// stale-frame draw followed by a second fresh-frame draw.
///
/// Lives on `app_msg` (not `app`) because it's the common return type
/// of every dispatch path — keyboard handler, dispatcher, mouse
/// handler all share it.
#[derive(Clone, Copy, PartialEq)]
pub enum InputEffect {
    None,
    Plugin,
    Map,
}

/// Outcome of handing a raw key event to the focused surface via
/// [`UiState::deliver_key`](crate::ui::UiState::deliver_key). The
/// host routes on this: `Passthrough` falls through to the global
/// fallback chain; `Consumed` is absorbed by the surface; `Run` is an
/// `AppMsg` for the caller to dispatch next.
pub enum KeyDelivery {
    /// Focus had no claim (`Focus::Map`) or the focused plugin
    /// returned `Pass`. Caller should try the global fallback chain.
    Passthrough,
    /// Focused surface consumed the key; no `AppMsg` to run.
    Consumed,
    /// Focused surface emitted an `AppMsg` — caller should
    /// `dispatch` it.
    Run(AppMsg),
}

/// Bundle of borrows every dispatcher entry point needs. Bundling
/// them into one struct keeps call sites tidy — call `dispatch(msg,
/// &mut ctx)` instead of threading four separate references through
/// every layer.
///
/// Fields are public so sites that already hold the individual pieces
/// (e.g. `app::App::run`) can build the ctx in one place and reuse it
/// for each dispatch in a loop iteration.
pub struct DispatchCtx<'a> {
    pub map: &'a mut MapState,
    pub ui: &'a mut UiState,
    pub render_handle: &'a RenderHandle,
    /// Read by `OpenPalette` (key hints in the default provider) and
    /// available to future messages that want to reason about key
    /// bindings. Other arms leave it alone.
    pub keymap: &'a KeyMap,
}

/// Apply an `AppMsg` to the app. Thin router: each arm delegates to a
/// single domain method that encapsulates the transition.
pub fn dispatch(msg: AppMsg, ctx: &mut DispatchCtx<'_>) -> InputEffect {
    match msg {
        AppMsg::Map(action) => {
            if ctx.map.process_action(&action) {
                InputEffect::Map
            } else {
                InputEffect::None
            }
        }
        AppMsg::Jump(loc) => {
            ctx.map.jump_to(loc);
            InputEffect::Map
        }
        AppMsg::Ui(action) => {
            ctx.ui.apply(action, ctx.render_handle);
            InputEffect::Map
        }
        AppMsg::ActivatePlugin(tag) => {
            ctx.ui.activate_plugin(&tag, ctx.map.center());
            InputEffect::Plugin
        }
        AppMsg::CycleFocus(forward) => {
            if ctx.ui.cycle_focus(forward) {
                InputEffect::Plugin
            } else {
                InputEffect::None
            }
        }
        AppMsg::OpenPalette => {
            ctx.ui.open_palette(ctx.keymap);
            InputEffect::Plugin
        }
        AppMsg::Resize(cols, rows) => {
            ctx.map.resize(cols, rows);
            ctx.render_handle
                .request_resize(ctx.map.width(), ctx.map.height());
            InputEffect::Map
        }
    }
}
