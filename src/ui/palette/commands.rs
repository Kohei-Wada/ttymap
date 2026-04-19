//! Command table for the command palette.
//!
//! Each `Command` couples a human label with either a `core::Action`
//! (dispatched through the usual keymap path), an `Activate(tag)`
//! directive (equivalent to pressing that plugin's activation key),
//! or a `SetTheme(ThemeId)` directive for runtime theme switching.
//! The palette enumerates these on open and filters them by query.

use crate::color_palette::ThemeId;
use crate::core::Action;

#[derive(Debug, Clone)]
pub enum CommandKind {
    Action(Action),
    Activate(String),
    SetTheme(ThemeId),
}

#[derive(Debug, Clone)]
pub struct Command {
    pub label: String,
    /// Pre-rendered key hint shown on the right edge of the row
    /// (e.g. `"j, Down"` or `"/"` for plugin activations). Empty means
    /// "no key bound" — the command is still runnable via the palette.
    pub keys: String,
    pub kind: CommandKind,
}

/// Static list of `(label, Action)` covering every map-level action the
/// core exposes. Kept here (not in `core::Action`) because labels are a
/// UI concern.
pub const ACTIONS: &[(&str, Action)] = &[
    ("Pan left", Action::PanLeft),
    ("Pan right", Action::PanRight),
    ("Pan up", Action::PanUp),
    ("Pan down", Action::PanDown),
    ("Pan left (fast)", Action::PanLeftFast),
    ("Pan right (fast)", Action::PanRightFast),
    ("Pan up (half)", Action::PanUpHalf),
    ("Pan down (half)", Action::PanDownHalf),
    ("Zoom in", Action::ZoomIn),
    ("Zoom out", Action::ZoomOut),
    ("Zoom to world", Action::ZoomToWorld),
    ("Reset position", Action::ResetPosition),
    ("Redraw", Action::Redraw),
    ("Quit", Action::Quit),
];
