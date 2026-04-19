//! Search widget — center popup for forward geocoding.
//!
//! Self-contained: owns its UI state, HTTP wrapper, and key dispatch.
//! `app.rs` sees it only through the [`Widget`](super::Widget) trait.

pub mod panel;
mod service;
mod state;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::core::Action;
use crate::geo::LonLat;
use crate::shared::nominatim::NominatimClient;

use service::SearchService;
use state::{Outcome, SearchState};

use super::{Widget, WidgetAction};

pub use panel::render_panel;

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
}

impl Widget for SearchWidget {
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        _center: LonLat,
    ) -> WidgetAction {
        if !self.state.is_active() {
            return WidgetAction::Pass;
        }
        match self.state.handle_key(code, modifiers) {
            Outcome::None | Outcome::Consumed => WidgetAction::Consumed,
            Outcome::Jump(loc) => WidgetAction::Jump(loc),
            Outcome::Submit(query) => {
                self.service.search(&query);
                WidgetAction::Consumed
            }
        }
    }

    fn handle_action(&mut self, action: &Action, _center: LonLat) -> bool {
        if *action == Action::SearchOpen {
            self.state.open();
            true
        } else {
            false
        }
    }

    fn poll(&mut self) -> bool {
        if let Some(results) = self.service.poll() {
            self.state.set_candidates(results);
            true
        } else {
            false
        }
    }
}
