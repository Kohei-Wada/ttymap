//! `MapAction` enum — map-level commands executed by `MapState`.
//!
//! Produced by the keyboard handler via the keymap lookup; consumed
//! only by `MapState::process_action`. Plugin activation lives outside
//! this enum — widgets register their own activation keys at startup
//! and are invoked directly by the keyboard handler, so `MapAction`
//! never carries UI-widget names.

use serde::{Deserialize, Serialize};

use crate::geo::LonLat;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MapAction {
    PanUp,
    PanDown,
    PanLeft,
    PanRight,
    PanLeftFast,
    PanRightFast,
    PanUpHalf,
    PanDownHalf,
    /// Zoom by `dir` steps. `+1` = `zoom_in`, `-1` = `zoom_out`; the
    /// integer is `dir * zoom_step` when applied, so larger magnitudes
    /// remain a free axis for future fast-zoom bindings without
    /// adding new variants.
    ZoomBy(i8),
    ZoomToWorld,
    ResetPosition,
    /// Continuous pan by terminal-cell deltas — produced by mouse drag.
    /// `Pan*` above are discrete, keybinding-friendly steps; this is
    /// the arbitrary-delta version the mouse needs.
    PanCells(i16, i16),
    /// Zoom toward a screen anchor — produced by mouse scroll wheel.
    /// The anchor is expressed in cells relative to screen center;
    /// `dir` follows the same convention as [`ZoomBy`] (`+1` zooms
    /// in, `-1` zooms out).
    ZoomAt {
        anchor_dx: f64,
        anchor_dy: f64,
        dir: i8,
    },
    /// Re-centre the map on a specific location. Produced by search,
    /// the here-plugin, and `ttymap.map:jump` from any Lua plugin — anything
    /// that yields a `LonLat` and wants to move the view there.
    Jump(LonLat),
    /// Set zoom directly (clamped to the map's `[min_zoom, max_zoom]`).
    /// Produced by `ttymap.map:zoom(level)` from Lua. Programmatic-only
    /// — not bound from `[keymap]` — so it lives outside `all_listed`.
    SetZoom(f64),
    /// Composite recenter + zoom in one dispatch. Produced by
    /// `ttymap.map:fly_to(lon, lat, zoom)`. Saves a round-trip vs.
    /// emitting `Jump` and `SetZoom` separately, which would render
    /// twice (intermediate frame at the new centre but old zoom).
    FlyTo {
        center: LonLat,
        zoom: f64,
    },
}

impl MapAction {
    /// Human-readable label used by the command palette and the help
    /// overlay. Mouse-only variants (`PanCells`, `ZoomAt`) return
    /// `""` since they are not exposed in UI listings. `label()` is
    /// the single source of truth; keep exhaustive so adding a
    /// variant triggers a compile error here.
    pub fn label(&self) -> &'static str {
        match self {
            MapAction::PanUp => "Pan up",
            MapAction::PanDown => "Pan down",
            MapAction::PanLeft => "Pan left",
            MapAction::PanRight => "Pan right",
            MapAction::PanLeftFast => "Pan left (fast)",
            MapAction::PanRightFast => "Pan right (fast)",
            MapAction::PanUpHalf => "Pan up (half)",
            MapAction::PanDownHalf => "Pan down (half)",
            MapAction::ZoomBy(1) => "Zoom in",
            MapAction::ZoomBy(-1) => "Zoom out",
            MapAction::ZoomBy(_) => "",
            MapAction::ZoomToWorld => "Zoom to world",
            MapAction::ResetPosition => "Reset position",
            MapAction::PanCells(..)
            | MapAction::ZoomAt { .. }
            | MapAction::Jump(_)
            | MapAction::SetZoom(_)
            | MapAction::FlyTo { .. } => "",
        }
    }

    /// Every `MapAction` variant surfaced in UI listings (command palette,
    /// help overlay). Excludes mouse-only variants. Adding a new
    /// keymap-bindable variant means adding it here.
    pub fn all_listed() -> &'static [MapAction] {
        &[
            MapAction::PanLeft,
            MapAction::PanRight,
            MapAction::PanUp,
            MapAction::PanDown,
            MapAction::PanLeftFast,
            MapAction::PanRightFast,
            MapAction::PanUpHalf,
            MapAction::PanDownHalf,
            MapAction::ZoomBy(1),
            MapAction::ZoomBy(-1),
            MapAction::ZoomToWorld,
            MapAction::ResetPosition,
        ]
    }

    /// Stable, snake_case name used as the TOML key in `[keymap]`
    /// (e.g. `pan_left = ["h", "Left"]`). Mouse-only variants and
    /// `Jump` return `""` since they cannot be rebound from config.
    /// Exhaustive so adding a variant is a compile error.
    pub fn config_name(&self) -> &'static str {
        match self {
            MapAction::PanUp => "pan_up",
            MapAction::PanDown => "pan_down",
            MapAction::PanLeft => "pan_left",
            MapAction::PanRight => "pan_right",
            MapAction::PanLeftFast => "pan_left_fast",
            MapAction::PanRightFast => "pan_right_fast",
            MapAction::PanUpHalf => "pan_up_half",
            MapAction::PanDownHalf => "pan_down_half",
            MapAction::ZoomBy(1) => "zoom_in",
            MapAction::ZoomBy(-1) => "zoom_out",
            MapAction::ZoomBy(_) => "",
            MapAction::ZoomToWorld => "zoom_to_world",
            MapAction::ResetPosition => "reset_position",
            MapAction::PanCells(..)
            | MapAction::ZoomAt { .. }
            | MapAction::Jump(_)
            | MapAction::SetZoom(_)
            | MapAction::FlyTo { .. } => "",
        }
    }

    /// Reverse of [`config_name`]: resolve a TOML key back to its
    /// `MapAction`. Only listed (rebindable) variants match; unknown
    /// names yield `None`.
    pub fn from_config_name(name: &str) -> Option<MapAction> {
        Self::all_listed()
            .iter()
            .find(|a| a.config_name() == name)
            .cloned()
    }
}
