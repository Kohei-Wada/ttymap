//! UI-level action vocabulary.
//!
//! `UiAction` is a category-level enum (symmetric with `map::Action`)
//! so the palette, plugins, and external control surfaces don't have
//! to know about every concrete UI mutation. The dispatcher
//! ([`apply`]) is the single place that knows how each variant
//! reshapes `UiState`. Today: theme switching; tomorrow: language /
//! export / whatever we need a toplevel UI command for.
//!
//! Wrapped by [`crate::command::Command::Ui`] so it rides the same
//! RPC surface as map actions and plugin activation.

use crate::color_palette::ThemeId;
use crate::map::render::thread::RenderHandle;
use crate::ui::UiState;

/// A single UI-level mutation request.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// Switch the running theme. Rebuilds the styler (on the render
    /// thread) and the UI color set.
    SetTheme(ThemeId),
}

/// Apply a `UiAction` to the app's UI state. Called by the central
/// command dispatcher — not directly by palette / plugins.
pub fn apply(action: UiAction, ui: &mut UiState, render_handle: &RenderHandle) {
    match action {
        UiAction::SetTheme(new_id) => {
            ui.theme_id = new_id;
            crate::theme::apply(new_id, &mut ui.theme, render_handle);
        }
    }
}
