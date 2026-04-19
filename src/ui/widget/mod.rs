//! UI widgets — self-contained components with their own state and rendering.
//!
//! Interactive widgets (search, help, wiki) implement the [`Widget`]
//! trait so `keyboard.rs` can dispatch events to them uniformly without
//! hard-coding the per-widget `Action` mapping. Focus — which widget
//! currently owns the keyboard — is tracked on `UiState.focus` and
//! mutated through [`WidgetCtx::focus`] from handler methods.

pub mod help;
pub mod map;
pub mod overlay;
pub mod search;
pub mod wiki;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::core::Action;
use crate::geo::LonLat;
use crate::ui::focus::Focus;

/// Outcome of a widget seeing a raw key event.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetAction {
    /// Key is not for this widget. Iteration should try the next widget
    /// and, if none claim it, the global keymap.
    Pass,
    /// Key consumed by the widget. App should redraw.
    Consumed,
    /// Widget wants the map recentered on this location.
    Jump(LonLat),
}

/// Context passed to widget handler methods. Exposes shared app state
/// the widget may need to read (current map center) or mutate (focus).
/// Keeping this in a struct lets us grow the surface (e.g. a command
/// queue, a notification channel) without resignalling every widget.
pub struct WidgetCtx<'a> {
    pub center: LonLat,
    pub focus: &'a mut Focus,
}

/// Interactive widget dispatched from the keyboard handler.
///
/// Widgets decide which keys and actions they consume; the keyboard
/// handler iterates them in priority order and never inspects
/// per-widget types. Focus is mutated through `ctx.focus` inside
/// handler methods.
pub trait Widget {
    /// Raw key event while this widget holds focus. The handler is
    /// only called when the dispatcher routes to it — widgets do not
    /// need to self-gate. Return `Pass` only when the key is
    /// deliberately delegated back to the global keymap (e.g. the
    /// wiki panel, which passes non-nav keys through).
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut WidgetCtx<'_>,
    ) -> WidgetAction;

    /// A global [`Action`] produced by the keymap. Returns `true` if
    /// the widget claimed it (e.g. `SearchOpen` on `SearchWidget`).
    /// Widgets typically transition `ctx.focus` here.
    fn handle_action(&mut self, action: &Action, ctx: &mut WidgetCtx<'_>) -> bool;

    /// Drain any async/background work. Returns `true` if state
    /// changed and the app should redraw.
    fn poll(&mut self) -> bool {
        false
    }
}
