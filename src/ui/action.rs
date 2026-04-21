//! UI-level action vocabulary.
//!
//! `UiAction` is a category-level enum (symmetric with `map::Action`)
//! so the palette, plugins, and external control surfaces don't have
//! to know about every concrete UI mutation. Today: theme switching;
//! tomorrow: language / export / whatever needs a toplevel UI command.
//!
//! Wrapped by [`crate::app_command::AppCommand::Ui`] so it rides the same
//! RPC surface as map actions and plugin activation. Application
//! lives on [`UiState::apply`](crate::ui::UiState::apply) — the UI
//! type owns the workflow; the dispatcher just calls it.

use crate::color_palette::ThemeId;

/// A single UI-level mutation request.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    /// Switch the running theme. Rebuilds the styler (on the render
    /// thread) and the UI color set.
    SetTheme(ThemeId),
    /// Mouse cursor moved to the given terminal cell. Emitted by the
    /// mouse router on every event so the overlay cursor readout goes
    /// through `dispatch` like every other user-intent state change.
    CursorMoved(u16, u16),
}
