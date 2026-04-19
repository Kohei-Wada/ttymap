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
use crate::theme::UiTheme;

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

    fn activate(&mut self, ctx: &mut PluginCtx) {
        // Non-modal: visible and focus are independent. Host drives
        // focus; here we only manage panel state. If the panel is
        // already visible, leave state alone — host may be reclaiming
        // focus (case 3) or closing via `close()` on toggle-off.
        if !self.state.is_active() {
            self.state.open();
            self.refresh(ctx.center);
        }
    }

    // Non-modal: keep the panel visible when focus leaves (default
    // deactivate is no-op). `close()` fires on user-initiated toggle-off
    // and tears down state.
    fn close(&mut self) {
        self.state.close();
    }

    fn visible(&self) -> bool {
        self.state.is_active()
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut PluginCtx,
    ) -> PluginAction {
        let outcome = self.state.handle_key(code, modifiers);
        // Focus release is host-driven: if `visible()` flips to false
        // (e.g. Esc closed the list), keyboard.rs releases for us.
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

    fn paint_on_map(&self, p: &mut crate::painter::MapPainter<'_>) {
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
