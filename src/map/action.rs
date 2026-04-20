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

impl Action {
    /// Human-readable label used by the command palette and the help
    /// overlay. Mouse-only variants (`PanCells`, `ZoomAt`) and the
    /// no-op `None` return `""` since they are not exposed in UI
    /// listings. `label()` is the single source of truth; keep
    /// exhaustive so adding a variant triggers a compile error here.
    pub fn label(&self) -> &'static str {
        match self {
            Action::None => "",
            Action::Quit => "Quit",
            Action::PanUp => "Pan up",
            Action::PanDown => "Pan down",
            Action::PanLeft => "Pan left",
            Action::PanRight => "Pan right",
            Action::PanLeftFast => "Pan left (fast)",
            Action::PanRightFast => "Pan right (fast)",
            Action::PanUpHalf => "Pan up (half)",
            Action::PanDownHalf => "Pan down (half)",
            Action::ZoomIn => "Zoom in",
            Action::ZoomOut => "Zoom out",
            Action::ZoomToWorld => "Zoom to world",
            Action::ResetPosition => "Reset position",
            Action::Redraw => "Redraw",
            Action::PanCells(..) | Action::ZoomAt { .. } => "",
        }
    }

    /// Every `Action` variant surfaced in UI listings (command palette,
    /// help overlay). Excludes mouse-only variants and the no-op
    /// `None`. Adding a new keymap-bindable variant means adding it
    /// here.
    pub fn all_listed() -> &'static [Action] {
        &[
            Action::PanLeft,
            Action::PanRight,
            Action::PanUp,
            Action::PanDown,
            Action::PanLeftFast,
            Action::PanRightFast,
            Action::PanUpHalf,
            Action::PanDownHalf,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomToWorld,
            Action::ResetPosition,
            Action::Redraw,
            Action::Quit,
        ]
    }
}
