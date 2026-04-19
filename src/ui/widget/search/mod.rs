//! Search widget — center popup for forward geocoding.
//!
//! Self-contained: owns its UI state, HTTP wrapper, and key dispatch.
//! `app.rs` sees it only through the [`Widget`](super::Widget) trait.

pub mod panel;
mod service;
mod state;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::core::Action;
use crate::shared::nominatim::NominatimClient;
use crate::ui::focus::Focus;
use crate::ui::theme::Theme;

use service::SearchService;
use state::{Outcome, SearchState};

use super::{Widget, WidgetAction, WidgetCtx};

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

    pub fn has_candidates(&self) -> bool {
        self.state.has_candidates()
    }
}

impl Widget for SearchWidget {
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut WidgetCtx<'_>,
    ) -> WidgetAction {
        let outcome = self.state.handle_key(code, modifiers);
        // Any outcome that closes the panel also releases focus.
        if !self.state.is_active() {
            *ctx.focus = Focus::Map;
        }
        match outcome {
            Outcome::None | Outcome::Consumed => WidgetAction::Consumed,
            Outcome::Jump(loc) => WidgetAction::Jump(loc),
            Outcome::Submit(query) => {
                self.service.search(&query);
                WidgetAction::Consumed
            }
        }
    }

    fn handle_action(&mut self, action: &Action, ctx: &mut WidgetCtx<'_>) -> bool {
        if *action == Action::SearchOpen {
            self.state.open();
            *ctx.focus = Focus::Widget("search".into());
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

    fn tag(&self) -> &str {
        "search"
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
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
