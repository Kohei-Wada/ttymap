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

use crate::geo::LonLat;
use crate::shared::throttle::Throttle;
use crate::ui::theme::Theme;
use crate::ui::widget::overlay::MarkerPoint;

use service::WikiService;
use state::{KeyOutcome, WikiState};

use super::{Widget, WidgetAction, WidgetCtx};
use crate::ui::focus::Focus;

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

    fn refresh(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.geosearch(center.lat, center.lon);
        }
    }
}

impl Widget for WikiWidget {
    fn tag(&self) -> &str {
        "wiki"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["i"]
    }

    fn activate(&mut self, ctx: &mut WidgetCtx<'_>) {
        // Focus, not internal state, is the source of truth for
        // whether wiki is currently open.
        if ctx.focus.is_widget("wiki") {
            self.state.close();
            *ctx.focus = Focus::Map;
        } else {
            self.state.open();
            self.refresh(ctx.center);
            *ctx.focus = Focus::Widget("wiki".into());
        }
    }

    fn deactivate(&mut self) {
        self.state.close();
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut WidgetCtx<'_>,
    ) -> WidgetAction {
        let outcome = self.state.handle_key(code, modifiers);
        // Wiki is non-modal: the panel can close on its own (Esc while
        // not in detail view), in which case focus returns to the map.
        if !self.state.is_active() {
            *ctx.focus = Focus::Map;
        }
        match outcome {
            KeyOutcome::None => WidgetAction::Pass,
            KeyOutcome::Consumed => WidgetAction::Consumed,
            KeyOutcome::JumpTo(loc) => WidgetAction::Jump(loc),
            KeyOutcome::Refresh => {
                self.refresh(ctx.center);
                WidgetAction::Consumed
            }
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

    fn render(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect, theme: &Theme) {
        panel::render_panel(self, f, area, theme);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.is_detail_open() {
            vec![
                ("C-n/C-p", "prev/next"),
                ("Enter/Esc", "back"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("?", "help"),
            ]
        } else {
            vec![
                ("C-n/C-p", "select"),
                ("Enter", "open"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("/", "search"),
                ("?", "help"),
            ]
        }
    }

    fn markers(&self, theme: &Theme) -> Vec<MarkerPoint> {
        if !self.state.is_active() {
            return Vec::new();
        }
        self.state
            .articles
            .iter()
            .enumerate()
            .map(|(i, a)| MarkerPoint {
                lon: a.lon,
                lat: a.lat,
                glyph: '●',
                fg: if i == self.state.selected {
                    theme.accent_alt
                } else {
                    theme.accent
                },
            })
            .collect()
    }
}
