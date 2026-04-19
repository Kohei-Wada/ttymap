//! Search widget — center popup for forward geocoding.
//!
//! Self-contained: owns its UI state, HTTP wrapper, and key dispatch.
//! `app.rs` sees it only through the [`Plugin`](super::Plugin) trait.

pub mod panel;
mod service;
mod state;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::shared::nominatim::NominatimClient;
use crate::ui::theme::UiTheme;

use service::SearchService;
use state::{Outcome, SearchState};

use super::{Plugin, PluginAction, PluginCtx};

pub struct SearchPlugin {
    pub(in crate::plugin::search) state: SearchState,
    service: SearchService,
}

impl SearchPlugin {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            state: SearchState::new(),
            service: SearchService::new(nominatim),
        }
    }

    pub fn has_candidates(&self) -> bool {
        self.state.has_candidates()
    }
}

impl Plugin for SearchPlugin {
    fn tag(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search location"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["/"]
    }

    fn activate(&mut self, ctx: &mut PluginCtx<'_>) {
        self.state.open();
        ctx.focus.take("search");
    }

    fn deactivate(&mut self) {
        self.state.close();
    }

    fn visible(&self) -> bool {
        self.state.is_active()
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut PluginCtx<'_>,
    ) -> PluginAction {
        let outcome = self.state.handle_key(code, modifiers);
        if !self.state.is_active() {
            ctx.focus.release();
        }
        match outcome {
            Outcome::None | Outcome::Consumed => PluginAction::Consumed,
            Outcome::Jump(loc) => PluginAction::Jump(loc),
            Outcome::Submit(query) => {
                self.service.search(&query);
                PluginAction::Consumed
            }
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

    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    }
}
