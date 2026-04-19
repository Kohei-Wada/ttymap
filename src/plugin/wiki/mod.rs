//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Self-contained: `app.rs` sees it only through the
//! [`Plugin`](super::Plugin) trait. The HTTP client and async wrapper
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
use crate::ui::theme::UiTheme;

use service::WikiService;
use state::{KeyOutcome, WikiState};

use super::{Plugin, PluginAction, PluginCtx};

pub struct WikiPlugin {
    pub(in crate::plugin::wiki) state: WikiState,
    service: WikiService,
    throttle: Throttle,
}

impl WikiPlugin {
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

impl Plugin for WikiPlugin {
    fn tag(&self) -> &str {
        "wiki"
    }

    fn description(&self) -> &str {
        "Toggle wiki"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["i"]
    }

    fn activate(&mut self, ctx: &mut PluginCtx<'_>) {
        // Non-modal: visible and focus are independent.
        if !self.state.is_active() {
            // Not visible → open and take focus.
            self.state.open();
            self.refresh(ctx.center);
            ctx.focus.take("wiki");
        } else if ctx.focus.is_plugin("wiki") {
            // Visible and focused → toggle off.
            self.state.close();
            ctx.focus.release();
        } else {
            // Visible but another plugin has focus → reclaim focus.
            ctx.focus.take("wiki");
        }
    }

    // Non-modal: keep the panel visible when focus leaves. The
    // default no-op is fine.

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
        // Wiki is non-modal: the panel can close on its own (Esc while
        // not in detail view), in which case focus returns to the
        // previous holder (or the map if there wasn't one).
        if !self.state.is_active() {
            ctx.focus.release();
        }
        match outcome {
            KeyOutcome::None => PluginAction::Pass,
            KeyOutcome::Consumed => PluginAction::Consumed,
            KeyOutcome::JumpTo(loc) => PluginAction::Jump(loc),
            KeyOutcome::Refresh => {
                self.refresh(ctx.center);
                PluginAction::Consumed
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

    fn render(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect, theme: &UiTheme) {
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

    fn paint_on_map(&self, p: &mut crate::ui::painter::MapPainter<'_>) {
        if !self.state.is_active() {
            return;
        }
        let (primary, accent) = {
            let theme = p.theme();
            (theme.accent, theme.accent_alt)
        };
        for (i, a) in self.state.articles.iter().enumerate() {
            let fg = if i == self.state.selected {
                accent
            } else {
                primary
            };
            p.point(
                crate::geo::LonLat {
                    lon: a.lon,
                    lat: a.lat,
                },
                '●',
                fg,
            );
        }
    }
}
