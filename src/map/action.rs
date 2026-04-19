//! `Action` enum — map-level commands executed by `MapState`.
//!
//! Produced by the keyboard handler via the keymap lookup; consumed
//! only by `MapState::process_action`. Plugin activation lives outside
//! this enum — widgets register their own activation keys at startup
//! and are invoked directly by the keyboard handler, so `Action`
//! never carries UI-widget names.

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
    /// Continuous pan by terminal-cell deltas — produced by mouse drag.
    /// `Pan*` above are discrete, keybinding-friendly steps; this is
    /// the arbitrary-delta version the mouse needs.
    PanCells(i16, i16),
    /// Zoom toward a screen anchor — produced by mouse scroll wheel.
    /// The anchor is expressed in cells relative to screen center;
    /// `zoom_in == true` zooms in by one step, else out.
    ZoomAt {
        anchor_dx: f64,
        anchor_dy: f64,
        zoom_in: bool,
    },
}
