//! Search state — input buffer, candidate list, selection.
//!
//! `app.rs` drives lifecycle (open, set_candidates) and forwards keys
//! through `handle_key`. The panel renderer only reads.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::nominatim::SearchResult;

#[derive(Debug, Clone, PartialEq)]
pub enum SearchAction {
    None,
    Submit(String),
    Select(usize),
    Cancel,
}

pub struct SearchState {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) candidates: Vec<SearchResult>,
    pub(super) selected: usize,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            candidates: Vec::new(),
            selected: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    pub fn open(&mut self) {
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
        self.active = true;
    }

    pub fn set_candidates(&mut self, candidates: Vec<SearchResult>) {
        self.candidates = candidates;
        self.selected = 0;
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> SearchAction {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && code == KeyCode::Char('p'));
            let down = matches!(code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && code == KeyCode::Char('n'));

            return if code == KeyCode::Esc {
                self.active = false;
                self.candidates.clear();
                SearchAction::Cancel
            } else if code == KeyCode::Enter {
                self.active = false;
                let idx = self.selected;
                self.candidates.clear();
                SearchAction::Select(idx)
            } else if up {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                SearchAction::None
            } else if down {
                if self.selected + 1 < self.candidates.len() {
                    self.selected += 1;
                }
                SearchAction::None
            } else {
                SearchAction::None
            };
        }

        match code {
            KeyCode::Esc => {
                self.active = false;
                SearchAction::Cancel
            }
            KeyCode::Enter => {
                if self.query.is_empty() {
                    self.active = false;
                    SearchAction::Cancel
                } else {
                    SearchAction::Submit(self.query.clone())
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                SearchAction::None
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                SearchAction::None
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                SearchAction::None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                SearchAction::None
            }
            _ => SearchAction::None,
        }
    }
}
