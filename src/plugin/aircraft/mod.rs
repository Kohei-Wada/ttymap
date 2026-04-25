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
mod panel;
mod state;

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;

use crate::config::Config;
use crate::plugin_api::prelude::*;

use component::AircraftComponent;
use state::{AircraftHandle, AircraftState};

/// Aircraft plugin config (`[aircraft]` in config.toml).
#[derive(Deserialize)]
#[serde(default)]
pub struct AircraftConfig {
    /// Min seconds between fetches. OpenSky's anonymous quota is
    /// ~4000 credits/day; bbox calls cost 1 each, so 12 s (=5/min)
    /// is well under the cap.
    pub interval_secs: u64,
    /// Half-side of the bounding box (degrees) sent to OpenSky.
    /// Larger keeps markers visible after small pans without a
    /// re-fetch; smaller keeps the response compact.
    pub fetch_half_deg: f64,
}

impl Default for AircraftConfig {
    fn default() -> Self {
        Self {
            interval_secs: 12,
            fetch_half_deg: 5.0,
        }
    }
}

/// Wire the aircraft plugin into the registrar. Palette-only
/// activation for now — no dedicated key — so we don't fight the
/// existing `a → ZoomIn` binding.
pub fn register(config: &Config, r: &mut Registrar) {
    let cfg: AircraftConfig = config.plugin("aircraft");
    let state: AircraftHandle = Rc::new(RefCell::new(AircraftState::new(cfg)));
    r.add_toggle("Toggle aircraft", "", move |ctx| {
        AircraftComponent::new(state.clone(), ctx.center)
    });
}
