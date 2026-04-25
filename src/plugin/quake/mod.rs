//! Quake plugin — recent earthquakes from the USGS public feed.
//!
//! Activated via the command palette ("Toggle quakes"). Fetches the
//! 24-hour M2.5+ summary on push and refreshes every 5 minutes;
//! markers disappear when the panel is popped.
//!
//! Each quake currently renders as a single cell — `·` for routine
//! tremors and `✸` (with the alt accent colour) for newsworthy
//! M5+. Magnitude / depth want graduated styling beyond a binary
//! threshold; that is on the MapApi-primitive backlog (graded color,
//! point size, label) rather than something this plugin should
//! invent locally.
//!
//! On first successful fetch the map auto-jumps to the highest-
//! magnitude quake so the user always lands somewhere meaningful —
//! matching the ISS plugin's "you toggled this on, see the thing
//! immediately" UX.
//!
//! ## Layout
//!
//! - [`state`] — `QuakeState`: polled feed + cached list + lookup
//! - [`component`] — `QuakeComponent`: marker rendering, auto-jump
//! - [`usgs`] — HTTP client + GeoJSON parser (private)

mod component;
mod state;
mod usgs;

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;

use crate::config::Config;
use crate::plugin_api::prelude::*;

use component::QuakeComponent;
use state::{QuakeHandle, QuakeState};

/// Quake plugin config (`[quake]` in config.toml).
#[derive(Deserialize)]
#[serde(default)]
pub struct QuakeConfig {
    /// Min seconds between feed pulls. USGS itself updates ≈1/min;
    /// 5 min keeps load polite while picking up new events promptly.
    pub interval_secs: u64,
}

impl Default for QuakeConfig {
    fn default() -> Self {
        Self { interval_secs: 300 }
    }
}

/// Wire the quake plugin into the registrar. Palette-only activation.
pub fn register(config: &Config, r: &mut Registrar) {
    let cfg: QuakeConfig = config.plugin("quake");
    let state: QuakeHandle = Rc::new(RefCell::new(QuakeState::new(cfg)));
    r.add_toggle("Toggle quakes", "", move |_| {
        QuakeComponent::new(state.clone())
    });
}
