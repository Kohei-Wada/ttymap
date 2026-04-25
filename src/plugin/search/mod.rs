//! Search plugin — center popup for forward geocoding.
//!
//! Under the compositor model, search is an **ephemeral** component:
//! a fresh instance is pushed onto the stack when the user hits `/`
//! (or selects it from the palette); it's popped when the user
//! confirms a result, cancels, or submits an empty query. No
//! per-open state to reset because the object itself is discarded
//! and rebuilt.
//!
//! ## Layout
//!
//! - [`component`] — `SearchComponent`: query input + candidate list
//! - [`panel`] — center-popup render (consumed by `component::render`)

mod component;
pub mod panel;

pub use component::SearchComponent;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::plugin_api::nominatim::NominatimClient;
use crate::plugin_api::prelude::*;

/// Wire the search plugin into the registrar. Adds:
/// - activation on `/` → push a fresh [`SearchComponent`]
/// - palette entry so the picker can reach it
pub fn register(nominatim: Arc<NominatimClient>, r: &mut Registrar) {
    let nominatim_for_key = nominatim.clone();
    r.bind(KeyCode::Char('/'), KeyModifiers::NONE, move |_| {
        SearchComponent::new(nominatim_for_key.clone())
    });
    r.add_spawn("Search location", "/", move |_| {
        SearchComponent::new(nominatim.clone())
    });
}
