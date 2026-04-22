//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Under the compositor model wiki is split into two surfaces sharing
//! one state handle:
//!
//! - [`WikiComponent`] — the focus-side modal panel. Pushed onto the
//!   compositor on activation, popped on close.
//! - [`WikiPainter`] — the always-on map markers. Registered in the
//!   app's painter list at startup; renders every frame regardless of
//!   whether the component is on the stack.
//!
//! Both hold an `Rc<RefCell<WikiState>>` so the marker list, current
//! selection, and the detail view survive panel open/close cycles and
//! stay in sync between the two views.

pub mod panel;
mod service;
mod wikipedia;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::debug;

use crate::app::AppMsg;
use crate::compositor::{
    Activation, Component, Context, EventResult, PaletteEntry, PaletteKind, Painter, Registrar,
};
use crate::geo::LonLat;
use crate::painter::MapPainter;
use crate::shared::throttle::Throttle;
use crate::theme::UiTheme;

use service::WikiService;
use wikipedia::WikiArticle;

/// Shared state for the wiki subsystem. Lives behind an
/// `Rc<RefCell<_>>` so both the focus-side ([`WikiComponent`]) and
/// the map-side ([`WikiPainter`]) views share one source of truth.
pub struct WikiState {
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

impl WikiState {
    pub fn new(language: &str, limit: u32) -> Self {
        Self {
            articles: Vec::new(),
            selected: 0,
            detail: None,
            service: WikiService::new(language, limit),
            throttle: Throttle::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    fn refresh(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.geosearch(center.lat, center.lon);
        }
    }

    /// Merge fresh fetch results into the existing list.
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

    /// Advance async fetch, merging new results when available.
    pub fn poll(&mut self) -> bool {
        if let Some(articles) = self.service.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.set_articles(articles);
            true
        } else {
            false
        }
    }
}

pub type WikiHandle = Rc<RefCell<WikiState>>;

/// Focus-side view of wiki. Holds no state of its own — state lives
/// in `WikiHandle`, so push/pop is cheap and can't desync from the
/// painter.
pub struct WikiComponent {
    state: WikiHandle,
}

impl WikiComponent {
    pub fn new(state: WikiHandle, center: LonLat) -> Self {
        // Trigger a refresh on (re)open so the list reflects the
        // user's current position. Replaces the old `activate` hook.
        state.borrow_mut().refresh(center);
        Self { state }
    }
}

impl Component for WikiComponent {
    fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> EventResult {
        let mut state = self.state.borrow_mut();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        // Self-toggle on the activation key.
        if event.code == KeyCode::Char('i') && event.modifiers == KeyModifiers::NONE {
            return EventResult::Close(Vec::new());
        }

        // Refresh is available even when the list is empty.
        if event.code == KeyCode::Char('r') {
            state.refresh(ctx.center);
            return EventResult::Consumed(Vec::new());
        }

        let up = (ctrl && event.code == KeyCode::Char('p')) || event.code == KeyCode::Up;
        let down = (ctrl && event.code == KeyCode::Char('n')) || event.code == KeyCode::Down;
        let exit_detail = matches!(
            event.code,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter
        );

        if state.articles.is_empty() {
            // Panel is open but has nothing yet — still swallow
            // widget-control keys so they don't fall through to the
            // keymap.
            if up || down || exit_detail {
                return EventResult::Consumed(Vec::new());
            }
            return EventResult::Ignored;
        }

        // ── Detail mode ─────────────────────────────────────────────
        if state.is_detail_open() {
            if exit_detail {
                state.detail = None;
                return EventResult::Consumed(Vec::new());
            }
            if up || down {
                let n = state.articles.len();
                state.selected = if up {
                    if state.selected == 0 { n - 1 } else { state.selected - 1 }
                } else {
                    (state.selected + 1) % n
                };
                let article = state.articles[state.selected].clone();
                let loc = LonLat { lat: article.lat, lon: article.lon };
                state.detail = Some(article);
                return EventResult::Consumed(vec![AppMsg::Jump(loc)]);
            }
            return EventResult::Consumed(Vec::new());
        }

        // ── List mode ───────────────────────────────────────────────
        if event.code == KeyCode::Enter {
            if let Some(article) = state.articles.get(state.selected) {
                let loc = LonLat { lat: article.lat, lon: article.lon };
                state.detail = Some(article.clone());
                return EventResult::Consumed(vec![AppMsg::Jump(loc)]);
            }
            return EventResult::Consumed(Vec::new());
        }
        if up || down {
            let n = state.articles.len();
            state.selected = if up {
                if state.selected == 0 { n - 1 } else { state.selected - 1 }
            } else {
                (state.selected + 1) % n
            };
            let article = &state.articles[state.selected];
            return EventResult::Consumed(vec![AppMsg::Jump(LonLat {
                lat: article.lat,
                lon: article.lon,
            })]);
        }
        if matches!(event.code, KeyCode::Esc | KeyCode::Backspace) {
            return EventResult::Consumed(Vec::new());
        }

        // Non-modal: let lower layers handle unknown keys.
        EventResult::Ignored
    }

    fn render(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect, theme: &UiTheme) {
        panel::render_panel(&self.state.borrow(), f, area, theme);
    }

    fn poll(&mut self) -> Vec<AppMsg> {
        self.state.borrow_mut().poll();
        Vec::new()
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.borrow().is_detail_open() {
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

/// Map-side view of wiki. Paints markers for every article every
/// frame, regardless of whether the panel is on the compositor stack.
pub struct WikiPainter {
    state: WikiHandle,
}

impl WikiPainter {
    pub fn new(state: WikiHandle) -> Self {
        Self { state }
    }
}

impl Painter for WikiPainter {
    fn paint(&self, p: &mut MapPainter<'_>) {
        let state = self.state.borrow();
        let (primary, accent) = {
            let theme = p.theme();
            (theme.accent, theme.accent_alt)
        };
        for (i, a) in state.articles.iter().enumerate() {
            let fg = if i == state.selected { accent } else { primary };
            p.point(
                LonLat { lon: a.lon, lat: a.lat },
                '●',
                fg,
            );
        }
    }
}

/// Wire wiki into the registrar. Creates the shared state once,
/// hands clones of the handle to both the painter (always on) and
/// the activation spawn closure (creates fresh `WikiComponent` on
/// each activation).
pub fn register(language: &str, limit: u32, r: &mut Registrar) {
    let state: WikiHandle = Rc::new(RefCell::new(WikiState::new(language, limit)));

    r.add_painter(Box::new(WikiPainter::new(state.clone())));

    {
        let state = state.clone();
        r.add_activation(Activation {
            code: KeyCode::Char('i'),
            modifiers: KeyModifiers::NONE,
            spawn: Box::new(move |ctx: &Context| -> Box<dyn Component> {
                Box::new(WikiComponent::new(state.clone(), ctx.center))
            }),
        });
    }

    {
        let state = state;
        r.add_palette_entry(PaletteEntry {
            label: "Toggle wiki".to_string(),
            hint: "i".to_string(),
            kind: PaletteKind::Spawn(Box::new(move |ctx: &Context| -> Box<dyn Component> {
                Box::new(WikiComponent::new(state.clone(), ctx.center))
            })),
        });
    }
}
