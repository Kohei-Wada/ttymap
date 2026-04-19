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
}
