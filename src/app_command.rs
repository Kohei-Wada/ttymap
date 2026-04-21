//! App-level command vocabulary + central dispatcher.
//!
//! `AppCommand` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async `pending_command`, and (one day) external
//! control surfaces like an HTTP/JSON-RPC front. Everyone speaks the
//! same vocabulary.
//!
//! This is the **Command pattern** (GoF): a closed enum of imperative
//! intents (`Map`, `Jump`, `CycleFocus`), each with exactly one
//! handler. *Not* an event/message bus — there is no broadcast and no
//! subscriber registration. Emitter → router (`dispatch`) → one domain
//! method per arm.
//!
//! Surface activation (palette open, plugin activate) intentionally
//! does *not* live here — those are focus transitions, expressed as
//! [`Effect::Open(SurfaceId)`] returned by a [`FocusSurface`] and
//! handled by [`FocusManager::open`](crate::focus::FocusManager::open)
//! directly. Keeping them off `AppCommand` means the focus state
//! machine isn't coupled to the dispatch table.
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
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Terminal resized — update the map viewport and the render
    /// thread's canvas dimensions. Arguments are the new terminal
    /// size in cells.
    Resize(u16, u16),
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
    /// Active theme — owned by `App`, mutated in-place by the
    /// `Ui(SetTheme)` arm. Pushed into the palette's internal cache
    /// (so the palette can highlight the active theme entry without
    /// taking it as a constructor arg) and used to derive `ui_theme`
    /// / render-thread `Styler` on a runtime switch.
    pub theme_id: &'a mut ThemeId,
    /// Derived UI colour set — kept in sync with `theme_id` by the
    /// `Ui(SetTheme)` arm. App passes a `&UiTheme` view of this into
    /// `ui::draw`.
    pub ui_theme: &'a mut UiTheme,
}

/// Apply an `AppCommand` to the app. Thin router: each arm delegates
/// to a single domain method that encapsulates the transition, and is
/// responsible for requesting a map redraw via [`request_map_redraw`]
/// when its effect changed the map frame. This keeps the "what
/// changed?" knowledge local to each arm instead of leaking out
/// through a separate return value.
pub fn dispatch(cmd: AppCommand, ctx: &mut DispatchCtx<'_>) {
    match cmd {
        AppCommand::Map(action) => {
            if ctx.map.process_action(&action) {
                request_map_redraw(ctx);
            }
        }
        AppCommand::Jump(loc) => {
            ctx.map.jump_to(loc);
            request_map_redraw(ctx);
        }
        AppCommand::Ui(UiAction::SetTheme(new_id)) => {
            // `SetTheme` re-derives both the UI colour cache and the
            // map styler from the new theme id. The render thread
            // gets the styler via message; we re-render so the change
            // is visible without waiting for another map event. The
            // palette's theme-picker entry reads `theme_id` via
            // `SurfaceCtx` on activation, so no surface-level push.
            *ctx.theme_id = new_id;
            let styler = Arc::new(Styler::new(new_id));
            *ctx.ui_theme = UiTheme::from_palette(styler.palette());
            ctx.render_handle.set_styler(styler);
            request_map_redraw(ctx);
        }
        AppCommand::Ui(UiAction::CursorMoved(col, row)) => {
            ctx.ui.overlay.set_cursor((col, row));
        }
        AppCommand::CycleFocus(forward) => {
            ctx.ui.focus.cycle(forward);
        }
        AppCommand::Resize(cols, rows) => {
            ctx.map.resize(cols, rows);
            ctx.render_handle
                .request_resize(ctx.map.width(), ctx.map.height());
            request_map_redraw(ctx);
        }
    }
}

/// Request a fresh map frame from the render thread and notify
/// passive widgets that the map recentered. Called by the dispatch
/// arms whose command actually changed the map frame. No-op after
/// shutdown (`map.is_running() == false`).
///
/// Wiki is intentionally not notified — Google-Maps-style, the
/// article list stays pinned to the query that produced it.
fn request_map_redraw(ctx: &mut DispatchCtx<'_>) {
    if !ctx.map.is_running() {
        return;
    }
    let viewport = ctx.map.viewport();
    ctx.render_handle.request_draw(viewport);
    ctx.ui.overlay.on_map_moved(viewport.center);
}
