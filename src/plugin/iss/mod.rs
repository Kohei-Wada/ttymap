//! ISS plugin — the International Space Station as a single moving
//! marker.
//!
//! Activated via the command palette ("Toggle ISS"). Polls open-notify
//! every 5 seconds for the current latitude / longitude and paints
//! one glyph at that point. The marker disappears when the panel is
//! popped because rendering is gated on stack presence.
//!
//! The ISS moves at roughly 7.66 km/s, so a multi-second refresh
//! produces visible motion across the map without hammering the
//! upstream API.
//!
//! ## Layout
//!
//! - [`state`] — `IssState`: polled feed + cached position
//! - [`component`] — `IssComponent`: marker + label + Enter-to-jump
//! - [`opennotify`] — HTTP client + JSON parser (private)

mod component;
mod opennotify;
mod panel;
mod state;

use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;

use crate::config::Config;
use crate::plugin_api::prelude::*;

use component::IssComponent;
use state::{IssHandle, IssState};

/// ISS plugin config (`[iss]` in config.toml).
#[derive(Deserialize)]
#[serde(default)]
pub struct IssConfig {
    /// Min seconds between fetches. ~5 s gives clearly-visible
    /// motion (38 km between samples) while staying polite.
    pub interval_secs: u64,
}

impl Default for IssConfig {
    fn default() -> Self {
        Self { interval_secs: 5 }
    }
}

/// Wire the ISS plugin into the registrar. Palette-only activation.
pub fn register(config: &Config, r: &mut Registrar) {
    let cfg: IssConfig = config.plugin("iss");
    let state: IssHandle = Rc::new(RefCell::new(IssState::new(cfg)));
    r.add_toggle("Toggle ISS", "", move |_| IssComponent::new(state.clone()));
}
