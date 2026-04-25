//! Aircraft plugin — live ADS-B markers from OpenSky Network.
//!
//! Activated via the command palette ("Toggle aircraft"). On push the
//! component kicks off a fetch around the current map centre; new
//! results refresh automatically every 12 seconds. Markers
//! disappear when the panel is popped because rendering is gated on
//! stack presence (`Component::paint_on_map`).
//!
//! Heading-aware glyphs, callsign labels, and on-screen detail are
//! deliberately out of scope for v1 — they are the plugin-side pain
//! points that should drive future MapApi primitives.
//!
//! ## Layout
//!
//! - [`state`] — `AircraftState`: polled feed + cached list
//! - [`component`] — `AircraftComponent`: marker rendering, polling
//! - [`opensky`] — HTTP client + JSON parser (private)

mod component;
mod opensky;
mod state;

use std::cell::RefCell;
use std::rc::Rc;

use crate::plugin_api::prelude::*;

use component::AircraftComponent;
use state::{AircraftHandle, AircraftState};

/// Wire the aircraft plugin into the registrar. Palette-only
/// activation for now — no dedicated key — so we don't fight the
/// existing `a → ZoomIn` binding.
pub fn register(r: &mut Registrar) {
    let state: AircraftHandle = Rc::new(RefCell::new(AircraftState::new()));
    r.add_toggle("Toggle aircraft", "", move |ctx| {
        AircraftComponent::new(state.clone(), ctx.center)
    });
}
