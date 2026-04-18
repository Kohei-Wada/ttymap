//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Self-contained: `app.rs` only interacts through [`WikiWidget`] and
//! [`WikiAction`]. The HTTP client and async wrapper are private to
//! this module.

pub mod panel;
mod service;
mod state;
mod wikipedia;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use log::debug;

use crate::geo::LonLat;
use crate::shared::throttle::Throttle;
use crate::ui::theme::Theme;
use crate::ui::widget::overlay::MarkerPoint;

use service::WikiService;
use state::{KeyOutcome, WikiState};

pub use panel::render_panel;

/// Action surfaced to the app event loop. `Refresh` is intentionally
/// absent — the widget handles its own fetches internally.
#[derive(Debug, Clone, PartialEq)]
pub enum WikiAction {
    /// Key not handled — let the global keymap see it.
    None,
    /// Key handled, no further effect beyond a redraw.
    Consumed,
    /// User selected an article — recenter the map.
    JumpTo(LonLat),
}

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

    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        center: LonLat,
    ) -> WikiAction {
        match self.state.handle_key(code, modifiers) {
            KeyOutcome::None => WikiAction::None,
            KeyOutcome::Consumed => WikiAction::Consumed,
            KeyOutcome::JumpTo(loc) => WikiAction::JumpTo(loc),
            KeyOutcome::Refresh => {
                self.refresh(center);
                WikiAction::Consumed
            }
        }
    }

    /// Drain any completed background fetches into state.
    /// Returns `true` if new articles arrived (caller should redraw).
    pub fn poll(&mut self) -> bool {
        if let Some(articles) = self.service.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.state.set_articles(articles);
            true
        } else {
            false
        }
    }

    fn refresh(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.geosearch(center.lat, center.lon);
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
