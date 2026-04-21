//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Self-contained: `app.rs` sees it only through the
//! [`Plugin`](super::Plugin) trait. The HTTP client and async wrapper
//! are private to this module.

pub mod panel;
mod service;
mod wikipedia;

use std::collections::HashSet;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use log::debug;

use crate::app_command::AppCommand;
use crate::focus::{Effect, FocusSurface, SurfaceCtx};
use crate::geo::LonLat;
use crate::shared::throttle::Throttle;
use crate::theme::UiTheme;

use service::WikiService;
use wikipedia::WikiArticle;

use super::Plugin;

pub struct WikiPlugin {
    pub(in crate::plugin::wiki) active: bool,
    pub(in crate::plugin::wiki) articles: Vec<WikiArticle>,
    pub(in crate::plugin::wiki) selected: usize,
    /// Snapshot of the article being viewed in detail mode. A copy
    /// (not an index) so it survives even if the candidate list is
    /// refreshed (e.g. after the map panned and new nearby articles
    /// loaded).
    pub(in crate::plugin::wiki) detail: Option<WikiArticle>,
    service: WikiService,
    throttle: Throttle,
}

impl WikiPlugin {
    pub fn new(language: &str, limit: u32) -> Self {
        Self {
            active: false,
            articles: Vec::new(),
            selected: 0,
            detail: None,
            service: WikiService::new(language, limit),
            throttle: Throttle::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub(in crate::plugin::wiki) fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    fn open(&mut self) {
        self.active = true;
        self.selected = 0;
        self.detail = None;
    }

    fn close_state(&mut self) {
        self.active = false;
        self.selected = 0;
        self.detail = None;
    }

    fn refresh(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.geosearch(center.lat, center.lon);
        }
    }

    /// Merge fresh fetch results into the existing list — keeps any
    /// article that is still in the new set (preserves selection
    /// stability across pans), drops the rest, then appends new
    /// arrivals.
    fn set_articles(&mut self, new_articles: Vec<WikiArticle>) {
        let new_titles: HashSet<String> = new_articles.iter().map(|a| a.title.clone()).collect();
        self.articles.retain(|a| new_titles.contains(&a.title));

        let existing_titles: HashSet<String> =
            self.articles.iter().map(|a| a.title.clone()).collect();
        for article in new_articles {
            if !existing_titles.contains(&article.title) {
                self.articles.push(article);
            }
        }

        if self.selected >= self.articles.len() {
            self.selected = self.articles.len().saturating_sub(1);
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

    fn activate(&mut self, ctx: SurfaceCtx) {
        // Non-modal: visible and focus are independent. Host drives
        // focus; here we only manage panel state. If the panel is
        // already visible, leave state alone — the host may be
        // reclaiming focus from another plugin's activation key.
        // Toggle-off is handled by `handle_key`'s self-absorption of
        // the `i` key, which calls `close_state` directly.
        if !self.active {
            self.open();
            self.refresh(ctx.center);
        }
    }

    // Non-modal: keep the panel visible when focus leaves (default
    // deactivate is no-op).

    fn poll(&mut self) -> bool {
        if let Some(articles) = self.service.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.set_articles(articles);
            true
        } else {
            false
        }
    }

    fn render(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    fn paint_on_map(&self, p: &mut crate::painter::MapPainter<'_>) {
        if !self.active {
            return;
        }
        let (primary, accent) = {
            let theme = p.theme();
            (theme.accent, theme.accent_alt)
        };
        for (i, a) in self.articles.iter().enumerate() {
            let fg = if i == self.selected { accent } else { primary };
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

/// Non-modal key dispatch. Returns `Effect::Pass` for keys the wiki
/// panel doesn't recognise so the background responder gets a chance
/// — but only when the panel is open. Closed panel = `Pass` for
/// everything.
///
/// Focus release is host-driven: if `is_visible()` flips to false
/// (e.g. Esc closed the list) `ui::router` releases for us.
impl FocusSurface for WikiPlugin {
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers, ctx: SurfaceCtx) -> Effect {
        if !self.active {
            return Effect::Pass;
        }

        // Self-toggle on the activation key. Surfaces own their own
        // lifecycle: pressing `i` again while the panel has focus
        // closes it directly (no round-trip through the background
        // responder + `FocusManager::open` toggle path). The router
        // auto-releases focus when `is_visible()` flips to false.
        if code == KeyCode::Char('i') && modifiers == KeyModifiers::NONE {
            self.close_state();
            return Effect::Consumed;
        }

        // Refresh is available even when the list is empty (e.g.
        // initial load returned nothing and the user wants to retry).
        if code == KeyCode::Char('r') {
            self.refresh(ctx.center);
            return Effect::Consumed;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let up = (ctrl && code == KeyCode::Char('p')) || code == KeyCode::Up;
        let down = (ctrl && code == KeyCode::Char('n')) || code == KeyCode::Down;
        let exit_detail = matches!(code, KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter);

        if self.articles.is_empty() {
            // Panel is open but has nothing yet — still swallow
            // widget-control keys so they don't fall through to the
            // global keymap.
            if up || down || exit_detail {
                return Effect::Consumed;
            }
            return Effect::Pass;
        }

        // ── Detail mode ─────────────────────────────────────────────
        if self.detail.is_some() {
            if exit_detail {
                self.detail = None;
                return Effect::Consumed;
            }
            if up || down {
                if up {
                    self.selected = if self.selected == 0 {
                        self.articles.len() - 1
                    } else {
                        self.selected - 1
                    };
                } else {
                    self.selected = (self.selected + 1) % self.articles.len();
                }
                let article = self.articles[self.selected].clone();
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article);
                return Effect::Run(AppCommand::Jump(loc));
            }
            return Effect::Consumed;
        }

        // ── List mode ───────────────────────────────────────────────
        if code == KeyCode::Enter {
            if let Some(article) = self.articles.get(self.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article.clone());
                return Effect::Run(AppCommand::Jump(loc));
            }
            return Effect::Consumed;
        }
        if up || down {
            if up {
                // Wrap around: top → bottom.
                self.selected = if self.selected == 0 {
                    self.articles.len() - 1
                } else {
                    self.selected - 1
                };
            } else {
                // Wrap around: bottom → top.
                self.selected = (self.selected + 1) % self.articles.len();
            }
            let article = &self.articles[self.selected];
            return Effect::Run(AppCommand::Jump(LonLat {
                lat: article.lat,
                lon: article.lon,
            }));
        }
        if matches!(code, KeyCode::Esc | KeyCode::Backspace) {
            return Effect::Consumed;
        }

        Effect::Pass
    }

    fn is_visible(&self) -> bool {
        self.active
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.is_detail_open() {
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
}
