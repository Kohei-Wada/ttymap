//! Search widget — center popup for forward geocoding.
//!
//! Self-contained: owns its UI state, HTTP wrapper, and key dispatch.
//! `app.rs` interacts only through [`SearchWidget`] and [`SearchAction`].

pub mod panel;
mod service;
mod state;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::geo::LonLat;
use crate::shared::nominatim::NominatimClient;

use service::SearchService;
use state::{Outcome, SearchState};

pub use panel::render_panel;

/// Action surfaced to the app event loop.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchAction {
    /// Key not handled (only returned when search isn't active, or
    /// an unrecognized key was pressed in query mode).
    None,
    /// Key handled, no further effect beyond a redraw.
    Consumed,
    /// User picked a candidate — recenter the map.
    Jump(LonLat),
}

pub struct SearchWidget {
    pub(in crate::ui::widget::search) state: SearchState,
    service: SearchService,
}

impl SearchWidget {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            state: SearchState::new(),
            service: SearchService::new(nominatim),
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    pub fn has_candidates(&self) -> bool {
        self.state.has_candidates()
    }

    pub fn open(&mut self) {
        self.state.open();
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> SearchAction {
        match self.state.handle_key(code, modifiers) {
            Outcome::None => SearchAction::None,
            Outcome::Consumed => SearchAction::Consumed,
            Outcome::Jump(loc) => SearchAction::Jump(loc),
            Outcome::Submit(query) => {
                self.service.search(&query);
                SearchAction::Consumed
            }
        }
    }

    /// Drain any completed forward-geocode results into the candidate list.
    /// Returns `true` if the list changed (caller should redraw).
    pub fn poll(&mut self) -> bool {
        if let Some(results) = self.service.poll() {
            self.state.set_candidates(results);
            true
        } else {
            false
        }
    }
}
