//! `Action` enum — commands the rest of the app can execute.
//! Produced by the keyboard handler (from raw key events via the
//! keymap + mode-transition shortcuts) and consumed by
//! `Core::process_action` and widget `handle_action` hooks.

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    PanUp,
    PanDown,
    PanLeft,
    PanRight,
    PanLeftFast,
    PanRightFast,
    PanUpHalf,
    PanDownHalf,
    ZoomIn,
    ZoomOut,
    ZoomToWorld,
    ResetPosition,
    Redraw,
    SearchOpen,
    HelpToggle,
    WikiToggle,
}
