//! Wiki widget — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Under the compositor model wiki is a single [`WikiComponent`]
//! with two rendering hooks:
//!
//! - `render` — the side panel (list / detail view), modal popup
//! - `paint_on_map` — article markers drawn on the map
//!
//! Both hooks fire only while the component is on the compositor
//! stack, so opening the panel shows the markers *and* the list, and
//! closing the panel removes both in step — no separate "paint
//! active?" flag to keep in sync.
//!
//! The persistent article list, selection, and detail state live in
//! [`WikiState`] behind an `Rc<RefCell<_>>` so a future second push
//! of the wiki panel (from the palette, say) sees the same cached
//! list without a re-fetch. The handle is owned by the spawn
//! closures in `register` and lives for the app's lifetime.

pub mod panel;
mod wikipedia;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::debug;

use crate::app::AppMsg;
use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Activation, Component, Context, PaletteEntry, PaletteKind, Registrar};
use crate::geo::LonLat;
use crate::map::MapApi;
use crate::plugin_api::PolledFeed;

use wikipedia::{WikiArticle, WikipediaClient};

/// Shared state for the wiki subsystem. Lives behind an
/// `Rc<RefCell<_>>` so every [`WikiComponent`] push (and any future
/// second view) shares one source of truth.
pub struct WikiState {
    pub(in crate::plugin::wiki) articles: Vec<WikiArticle>,
    pub(in crate::plugin::wiki) selected: usize,
    /// Snapshot of the article being viewed in detail mode. A copy
    /// (not an index) so it survives even if the candidate list is
    /// refreshed (e.g. after the map panned and new nearby articles
    /// loaded).
    pub(in crate::plugin::wiki) detail: Option<WikiArticle>,
    client: Arc<WikipediaClient>,
    limit: u32,
    feed: PolledFeed<Vec<WikiArticle>>,
}

impl WikiState {
    pub fn new(language: &str, limit: u32) -> Self {
        Self {
            articles: Vec::new(),
            selected: 0,
            detail: None,
            client: Arc::new(WikipediaClient::new(language)),
            limit,
            feed: PolledFeed::with_cooldown(Duration::from_secs(2)),
        }
    }

    pub fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        let limit = self.limit;
        self.feed
            .refresh(move || client.geosearch(center.lat, center.lon, limit));
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
        if let Some(articles) = self.feed.poll() {
            debug!("wiki: received {} articles", articles.len());
            self.set_articles(articles);
            true
        } else {
            false
        }
    }
}

pub type WikiHandle = Rc<RefCell<WikiState>>;

/// Wiki component — owns no state of its own. All state (articles,
/// selection, detail view) lives in the shared [`WikiHandle`], so
/// push/pop is cheap and the next open inherits the prior list.
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
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let mut state = self.state.borrow_mut();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        // Self-toggle on the activation key.
        if event.code == KeyCode::Char('i') && event.modifiers == KeyModifiers::NONE {
            win.close();
            return;
        }

        // Refresh is available even when the list is empty.
        if event.code == KeyCode::Char('r') {
            state.refresh(win.ctx().center);
            return;
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
            // keymap, but let everything else pass through (non-
            // modal behaviour).
            if !(up || down || exit_detail) {
                win.ignore();
            }
            return;
        }

        // ── Detail mode ─────────────────────────────────────────────
        if state.is_detail_open() {
            if exit_detail {
                state.detail = None;
                return;
            }
            if up || down {
                let n = state.articles.len();
                state.selected = if up {
                    if state.selected == 0 {
                        n - 1
                    } else {
                        state.selected - 1
                    }
                } else {
                    (state.selected + 1) % n
                };
                let article = state.articles[state.selected].clone();
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                state.detail = Some(article);
                win.emit(AppMsg::Jump(loc));
            }
            return;
        }

        // ── List mode ───────────────────────────────────────────────
        if event.code == KeyCode::Enter {
            if let Some(article) = state.articles.get(state.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                state.detail = Some(article.clone());
                win.emit(AppMsg::Jump(loc));
            }
            return;
        }
        if up || down {
            let n = state.articles.len();
            state.selected = if up {
                if state.selected == 0 {
                    n - 1
                } else {
                    state.selected - 1
                }
            } else {
                (state.selected + 1) % n
            };
            let article = &state.articles[state.selected];
            win.emit(AppMsg::Jump(LonLat {
                lat: article.lat,
                lon: article.lon,
            }));
            return;
        }
        if matches!(event.code, KeyCode::Esc | KeyCode::Backspace) {
            return;
        }

        // Non-modal: let lower layers handle unknown keys.
        win.ignore();
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(&self.state.borrow(), win);
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        let primary = p.accent_color();
        let highlight = p.accent_alt_color();
        for (i, a) in state.articles.iter().enumerate() {
            let fg = if i == state.selected {
                highlight
            } else {
                primary
            };
            p.point(
                LonLat {
                    lon: a.lon,
                    lat: a.lat,
                },
                '●',
                fg,
            );
        }
    }

    fn poll(&mut self, _win: &mut Window) {
        self.state.borrow_mut().poll();
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

/// Wire wiki into the registrar. Creates the shared state once; the
/// spawn closures for activation (`i`) and the palette entry both
/// clone the handle so every push shares the same persistent list.
pub fn register(language: &str, limit: u32, r: &mut Registrar) {
    let state: WikiHandle = Rc::new(RefCell::new(WikiState::new(language, limit)));

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
            kind: PaletteKind::Toggle(Box::new(move |ctx: &Context| -> Box<dyn Component> {
                Box::new(WikiComponent::new(state.clone(), ctx.center))
            })),
        });
    }
}
