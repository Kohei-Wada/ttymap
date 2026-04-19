//! Wiki domain state and the actions it surfaces to the app event loop.
//!
//! All mutations go through methods on [`WikiState`] and are triggered
//! from `app.rs`. The panel and marker overlay only read.

use crossterm::event::{KeyCode, KeyModifiers};

use super::wikipedia::WikiArticle;
use crate::geo::LonLat;

/// Outcome of feeding a key to [`WikiState::handle_key`].
/// `Refresh` is absorbed by the widget (it triggers a fetch) and
/// never surfaces to `app.rs` — that's why this enum is internal.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum KeyOutcome {
    None,
    Consumed,
    JumpTo(LonLat),
    Refresh,
}

/// Wiki panel state — article list, current selection, detail view,
/// visibility flag. Owned by `WikiWidget`; read by the side panel
/// renderer and the map marker overlay.
pub(super) struct WikiState {
    pub(super) active: bool,
    pub(super) articles: Vec<WikiArticle>,
    pub(super) selected: usize,
    /// Snapshot of the article being viewed in detail mode. A copy (not
    /// an index) so it survives even if the candidate list is refreshed
    /// (e.g. after the map panned and new nearby articles loaded).
    pub(super) detail: Option<WikiArticle>,
}

impl Default for WikiState {
    fn default() -> Self {
        Self::new()
    }
}

impl WikiState {
    pub fn new() -> Self {
        Self {
            active: false,
            articles: Vec::new(),
            selected: 0,
            detail: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    pub fn open(&mut self) {
        self.active = true;
        self.selected = 0;
        self.detail = None;
    }

    pub fn close(&mut self) {
        self.active = false;
        self.selected = 0;
        self.detail = None;
    }

    pub fn set_articles(&mut self, new_articles: Vec<WikiArticle>) {
        let new_titles: std::collections::HashSet<String> =
            new_articles.iter().map(|a| a.title.clone()).collect();

        // Keep existing that are still in new set
        self.articles.retain(|a| new_titles.contains(&a.title));

        // Add new ones not already present
        let existing_titles: std::collections::HashSet<String> =
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

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> KeyOutcome {
        if !self.active {
            return KeyOutcome::None;
        }

        // Refresh is available even when the list is empty (e.g. initial load
        // returned nothing and the user wants to retry).
        if code == KeyCode::Char('r') {
            return KeyOutcome::Refresh;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let up = (ctrl && matches!(code, KeyCode::Char('k') | KeyCode::Char('p')))
            || code == KeyCode::Up;
        let down = (ctrl && matches!(code, KeyCode::Char('j') | KeyCode::Char('n')))
            || code == KeyCode::Down;
        let exit_detail = matches!(code, KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter);

        if self.articles.is_empty() {
            // Panel is open but has nothing yet — still swallow widget-control
            // keys so they don't fall through to the global keymap.
            if up || down || exit_detail {
                return KeyOutcome::Consumed;
            }
            return KeyOutcome::None;
        }

        // ── Detail mode ─────────────────────────────────────────────────
        if self.detail.is_some() {
            if exit_detail {
                self.detail = None;
                return KeyOutcome::Consumed;
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
                return KeyOutcome::JumpTo(loc);
            }
            return KeyOutcome::Consumed;
        }

        // ── List mode ───────────────────────────────────────────────────
        if code == KeyCode::Enter {
            if let Some(article) = self.articles.get(self.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article.clone());
                return KeyOutcome::JumpTo(loc);
            }
            return KeyOutcome::Consumed;
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
            return KeyOutcome::JumpTo(LonLat {
                lat: article.lat,
                lon: article.lon,
            });
        }
        if matches!(code, KeyCode::Esc | KeyCode::Backspace) {
            return KeyOutcome::Consumed;
        }

        KeyOutcome::None
    }
}
