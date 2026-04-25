//! Info plugin — top-right always-on overlay showing the map's
//! current centre, the cursor's lat/lon, the zoom level, and the
//! reverse-geocoded place name.
//!
//! Replaces the legacy [`crate::ui::overlay::info::InfoOverlay`] —
//! its old `MapOverlay` trait + `OverlayManager` plumbing collapses
//! into a regular Component installed at startup via
//! `Registrar::add_overlay`. The plugin reads cursor position from
//! [`Context::cursor`](crate::compositor::Context) and centre / zoom
//! from `MapApi`; reverse geocoding uses Nominatim through a
//! [`PolledFeed`] tick.
//!
//! ## Layout
//!
//! - [`state`] — `InfoState`: cached place name + reverse-geocode feed
//! - [`component`] — `InfoComponent`: corner chrome render + poll

mod component;
mod state;

use std::sync::Arc;

use crate::plugin_api::nominatim::NominatimClient;
use crate::plugin_api::prelude::*;

use component::InfoComponent;
use state::InfoState;

/// Wire the info overlay into the registrar as an always-on plugin.
pub fn register(nominatim: Arc<NominatimClient>, r: &mut Registrar) {
    r.add_overlay(move |_ctx| InfoComponent::new(InfoState::new(nominatim.clone())));
}
