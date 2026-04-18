//! UI widgets — self-contained components with their own state and rendering.
//!
//! Interactive widgets (search, help, wiki) implement the [`Widget`]
//! trait so `app.rs` can dispatch events to them uniformly without
//! hard-coding the per-widget `Action` mapping.

pub mod help;
pub mod map;
pub mod overlay;
pub mod search;
pub mod wiki;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::core::input::Action;
use crate::geo::LonLat;

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

/// Interactive widget dispatched from `app.rs`.
///
/// Widgets decide which keys and actions they consume; `app.rs` iterates
/// them in priority order and never inspects per-widget types. Each
/// widget also exposes an inherent `is_active` method that other UI code
/// (layout, mouse gating) consults directly without needing this trait
/// in scope, so it isn't a trait requirement.
pub trait Widget {
    /// Raw key event. Widgets that aren't active return
    /// [`WidgetAction::Pass`]. Active widgets return `Pass` only when
    /// the key is deliberately delegated to the global keymap (e.g.
    /// wiki-panel non-nav keys).
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        center: LonLat,
    ) -> WidgetAction;

    /// A global [`Action`] produced by the keymap. Returns `true` if
    /// the widget claimed it (e.g. `SearchOpen` on `SearchWidget`).
    fn handle_action(&mut self, action: &Action, center: LonLat) -> bool;

    /// Drain any async/background work. Returns `true` if state
    /// changed and the app should redraw.
    fn poll(&mut self) -> bool {
        false
    }
}
