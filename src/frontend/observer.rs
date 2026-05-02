//! Observer hook surface for parties that want to react to Frontend
//! activity without Frontend knowing what they are.
//!
//! Frontend stores a single `Box<dyn AppObserver>` and calls these
//! methods at the relevant beats of the event loop. The Lua
//! subsystem ships an impl; in principle a telemetry layer, an
//! audit log, or an alternative scripting host could plug in the
//! same way. Frontend never imports any specific implementer — the
//! lua module is invisible from `frontend/mod.rs`.

use crate::frontend::UserIntent;
use crate::frontend::compositor::{Component, MapApi};
use crate::geo::LonLat;

/// Hook points fired by [`crate::frontend::Frontend`] during the
/// per-iteration loop. All methods default to no-op so an
/// implementer only overrides what it cares about.
pub trait AppObserver {
    /// Fired after Frontend dispatches a [`UserIntent`] (i.e. the
    /// state mutation has happened). Implementers typically
    /// translate selected variants into observable events for their
    /// own surface (e.g. fire `MAP_JUMPED` for `Action::Jump`).
    fn on_intent_dispatched(&self, _intent: &UserIntent) {}

    /// Fired when a fresh `MapFrame` is drained from the render
    /// thread into Frontend's cache.
    fn on_frame_ready(&self) {}

    /// Fired from `request_map_redraw` — the map view about to be
    /// drawn is `(center, zoom)`. Implementers can refresh any
    /// view-derived state in lockstep with the next render task.
    fn on_view_change(&self, _center: LonLat, _zoom: f64) {}

    /// Drain any pending components the observer wants to inject
    /// into the compositor stack. Called immediately before
    /// `compositor.poll`, so injected components participate in the
    /// same poll pass.
    fn drain_components(&self, _on_push: &mut dyn FnMut(Box<dyn Component>)) {}

    /// Pre-paint hook fired against the live `MapApi` from inside
    /// `ui::draw`, just before the compositor's `paint_on_map` pass.
    fn pre_paint_map(&self, _map: &mut MapApi<'_>) {}
}

/// No-op observer — useful for tests and headless callers.
impl AppObserver for () {}
