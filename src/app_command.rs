//! App-level command vocabulary + central dispatcher.
//!
//! `AppCommand` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async `pending_command`, and (one day) external
//! control surfaces like an HTTP/JSON-RPC front. Everyone speaks the
//! same vocabulary.
//!
//! This is the **Command pattern** (GoF): a closed enum of imperative
//! intents (`Pan`, `OpenPalette`, `ActivatePlugin`), each with exactly
//! one handler. *Not* an event/message bus — there is no broadcast and
//! no subscriber registration. Emitter → router (`dispatch`) → one
//! domain method per arm.
//!
//! [`dispatch`] is a **thin router**: each arm maps an `AppCommand` to a
//! single method on the domain type that owns the relevant state
//! (`UiState` / `MapState`). Those methods are where multi-step
//! invariants live (focus ↔ palette ↔ widgets transitions, etc.).
//! Adding a new command = one new `AppCommand` variant + one match arm +
//! the domain method it calls.

use std::sync::Arc;

use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState};
use crate::theme::UiTheme;
use crate::ui::UiState;
use crate::ui::action::UiAction;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; dispatched by
/// [`dispatch`] inside the input pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum AppCommand {
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
/// Lives on `app_command` (not `app`) because it's the common return type
/// of every dispatch path — keyboard handler, dispatcher, mouse
/// handler all share it.
#[derive(Clone, Copy, PartialEq)]
pub enum InputEffect {
    None,
    Plugin,
    Map,
}

/// Outcome of handing a key to a [`FocusSurface`] in the responder
/// chain. The router walks responders (focused surface →
/// [`BackgroundResponder`](crate::ui::router::background::BackgroundResponder))
/// until something other than `Pass` comes back.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Surface is not interested. Try the next responder in the chain
    /// (focused → background).
    Pass,
    /// Surface absorbed the key. No `AppCommand` to run.
    Consumed,
    /// Surface wants the host to run a command. The router returns it
    /// to `App::dispatch` for execution.
    Run(AppCommand),
}

/// Read-only context passed into [`FocusSurface::handle_key`]. Carries
/// the bits of shared state a surface needs but does not own (today:
/// the current map center, used by plugins for geo-relative actions).
/// Grow as new surface needs appear.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceCtx {
    pub center: LonLat,
}

/// Anything the router can deliver a focused-surface key event to.
/// Implemented by [`CommandPalette`](crate::ui::palette::CommandPalette)
/// and every [`Plugin`](crate::plugin::Plugin) (via a blanket adapter
/// in `plugin/mod.rs` that wraps their existing `handle_key`).
pub trait FocusSurface {
    fn handle_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        ctx: SurfaceCtx,
    ) -> Effect;
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
    /// Active theme — owned by `App`, mutated in-place by the
    /// `Ui(SetTheme)` arm. Read by `OpenPalette` (palette highlights
    /// the active theme entry) and used to derive `ui_theme` /
    /// render-thread `Styler` on a runtime switch.
    pub theme_id: &'a mut ThemeId,
    /// Derived UI colour set — kept in sync with `theme_id` by the
    /// `Ui(SetTheme)` arm. App passes a `&UiTheme` view of this into
    /// `ui::draw`.
    pub ui_theme: &'a mut UiTheme,
}

/// Apply an `AppCommand` to the app. Thin router: each arm delegates to a
/// single domain method that encapsulates the transition.
pub fn dispatch(cmd: AppCommand, ctx: &mut DispatchCtx<'_>) -> InputEffect {
    match cmd {
        AppCommand::Map(action) => {
            if ctx.map.process_action(&action) {
                InputEffect::Map
            } else {
                InputEffect::None
            }
        }
        AppCommand::Jump(loc) => {
            ctx.map.jump_to(loc);
            InputEffect::Map
        }
        AppCommand::Ui(action) => {
            apply_ui_action(action, ctx);
            InputEffect::Map
        }
        AppCommand::ActivatePlugin(tag) => {
            ctx.ui.activate_plugin(&tag, ctx.map.center());
            InputEffect::Plugin
        }
        AppCommand::CycleFocus(forward) => {
            if ctx.ui.cycle_focus(forward) {
                InputEffect::Plugin
            } else {
                InputEffect::None
            }
        }
        AppCommand::OpenPalette => {
            ctx.ui.open_palette(ctx.keymap, *ctx.theme_id);
            InputEffect::Plugin
        }
        AppCommand::Resize(cols, rows) => {
            ctx.map.resize(cols, rows);
            ctx.render_handle
                .request_resize(ctx.map.width(), ctx.map.height());
            InputEffect::Map
        }
    }
}

/// Apply a `UiAction` — today, theme switch. Owns the derivation:
/// `theme_id` → `UiTheme` (UI cache) + `Styler` (map render). Both
/// live at `App` level; this arm mutates them in place via the ctx.
fn apply_ui_action(action: UiAction, ctx: &mut DispatchCtx<'_>) {
    match action {
        UiAction::SetTheme(new_id) => {
            *ctx.theme_id = new_id;
            let styler = Arc::new(Styler::new(new_id));
            *ctx.ui_theme = UiTheme::from_palette(styler.palette());
            ctx.render_handle.set_styler(styler);
        }
    }
}
