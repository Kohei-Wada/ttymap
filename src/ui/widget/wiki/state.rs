//! Wiki domain state and the actions it surfaces to the app event loop.
//!
//! All mutations go through methods on [`WikiState`] and are triggered
//! from `app.rs`. The panel and marker overlay only read.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::geo::LonLat;
use crate::wikipedia::WikiArticle;

#[derive(Debug, Clone, PartialEq)]
pub enum WikiAction {
    None,
    JumpTo(LonLat),
}

/// Shared wiki state — article list, current selection, detail view,
/// panel visibility. Lives on `UiState`; both the side panel renderer
/// and the map marker overlay borrow it.
pub struct WikiState {
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

    pub fn toggle(&mut self) {
        self.active = !self.active;
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

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> WikiAction {
        if !self.active || self.articles.is_empty() {
            return WikiAction::None;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let up = (ctrl && matches!(code, KeyCode::Char('k') | KeyCode::Char('p')))
            || code == KeyCode::Up;
        let down = (ctrl && matches!(code, KeyCode::Char('j') | KeyCode::Char('n')))
            || code == KeyCode::Down;

        // ── Detail mode ─────────────────────────────────────────────────
        if self.detail.is_some() {
            if matches!(code, KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter) {
                self.detail = None;
                return WikiAction::None;
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
                return WikiAction::JumpTo(loc);
            }
            return WikiAction::None;
        }

        // ── List mode ───────────────────────────────────────────────────
        if code == KeyCode::Enter {
            if let Some(article) = self.articles.get(self.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article.clone());
                return WikiAction::JumpTo(loc);
            }
        } else if up || down {
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
            return WikiAction::JumpTo(LonLat {
                lat: article.lat,
                lon: article.lon,
            });
        }

        WikiAction::None
    }
}
