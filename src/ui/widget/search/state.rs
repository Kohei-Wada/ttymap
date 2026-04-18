//! Search state — input buffer, candidate list, selection.
//!
//! All key dispatch happens in [`SearchState::handle_key`]. `Submit` is
//! absorbed by the widget (triggers a forward geocode) and never
//! surfaces to `app.rs`; that's why [`Outcome`] is internal.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::geo::LonLat;
use crate::shared::nominatim::SearchResult;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Outcome {
    None,
    Consumed,
    Submit(String),
    Jump(LonLat),
}

pub(super) struct SearchState {
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
    pub(super) fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            candidates: Vec::new(),
            selected: 0,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    pub(super) fn open(&mut self) {
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
        self.active = true;
    }

    pub(super) fn set_candidates(&mut self, candidates: Vec<SearchResult>) {
        self.candidates = candidates;
        self.selected = 0;
    }

    pub(super) fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && code == KeyCode::Char('p'));
            let down = matches!(code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && code == KeyCode::Char('n'));

            return if code == KeyCode::Esc {
                self.active = false;
                self.candidates.clear();
                Outcome::Consumed
            } else if code == KeyCode::Enter {
                self.active = false;
                let loc = self.candidates[self.selected].location;
                self.candidates.clear();
                Outcome::Jump(loc)
            } else if up {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Outcome::Consumed
            } else if down {
                if self.selected + 1 < self.candidates.len() {
                    self.selected += 1;
                }
                Outcome::Consumed
            } else {
                Outcome::Consumed
            };
        }

        match code {
            KeyCode::Esc => {
                self.active = false;
                Outcome::Consumed
            }
            KeyCode::Enter => {
                if self.query.is_empty() {
                    self.active = false;
                    Outcome::Consumed
                } else {
                    Outcome::Submit(self.query.clone())
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                Outcome::Consumed
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                Outcome::Consumed
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                Outcome::Consumed
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                Outcome::Consumed
            }
            _ => Outcome::None,
        }
    }
}
