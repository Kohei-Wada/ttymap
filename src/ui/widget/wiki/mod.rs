//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Self-contained: `app.rs` sees it only through the
//! [`Widget`](super::Widget) trait. The HTTP client and async wrapper
//! are private to this module.

pub mod panel;
mod service;
mod state;
mod wikipedia;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use log::debug;

use crate::core::Action;
use crate::geo::LonLat;
use crate::shared::throttle::Throttle;
use crate::ui::theme::Theme;
use crate::ui::widget::overlay::MarkerPoint;

use service::WikiService;
use state::{KeyOutcome, WikiState};

use super::{Widget, WidgetAction};

pub use panel::render_panel;

pub struct WikiWidget {
    pub(in crate::ui::widget::wiki) state: WikiState,
    service: WikiService,
    throttle: Throttle,
}

impl WikiWidget {
    pub fn new(language: &str, limit: u32) -> Self {
        Self {
            state: WikiState::new(),
            service: WikiService::new(language, limit),
            throttle: Throttle::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    pub fn is_detail_open(&self) -> bool {
        self.state.is_detail_open()
    }

    /// Toggle panel visibility. Auto-fetches on open.
    pub fn toggle(&mut self, center: LonLat) {
        self.state.toggle();
        if self.state.is_active() {
            self.refresh(center);
        }
    }

    fn refresh(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.geosearch(center.lat, center.lon);
        }
    }
}

impl Widget for WikiWidget {
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        center: LonLat,
    ) -> WidgetAction {
        if !self.state.is_active() {
            return WidgetAction::Pass;
        }
        match self.state.handle_key(code, modifiers) {
            KeyOutcome::None => WidgetAction::Pass,
            KeyOutcome::Consumed => WidgetAction::Consumed,
            KeyOutcome::JumpTo(loc) => WidgetAction::Jump(loc),
            KeyOutcome::Refresh => {
                self.refresh(center);
                WidgetAction::Consumed
            }
        }
    }

    fn handle_action(&mut self, action: &Action, center: LonLat) -> bool {
        if *action == Action::WikiToggle {
            self.toggle(center);
            true
        } else {
            false
        }
    }

    fn poll(&mut self) -> bool {
        if let Some(articles) = self.service.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.state.set_articles(articles);
            true
        } else {
            false
        }
    }
}

/// Adapt the current article list into `MarkerPoint`s for `MarkersOverlay`.
/// Returns an empty vec when the panel is inactive so nothing is drawn.
pub fn marker_points(widget: &WikiWidget, theme: &Theme) -> Vec<MarkerPoint> {
    let state = &widget.state;
    if !state.is_active() {
        return Vec::new();
    }
    state
        .articles
        .iter()
        .enumerate()
        .map(|(i, a)| MarkerPoint {
            lon: a.lon,
            lat: a.lat,
            glyph: '●',
            fg: if i == state.selected {
                theme.accent_alt
            } else {
                theme.accent
            },
        })
        .collect()
}
