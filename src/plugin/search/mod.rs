//! Search widget — center popup for forward geocoding.
//!
//! Under the compositor model, search is an **ephemeral** component:
//! a fresh instance is pushed onto the stack when the user hits `/`
//! (or selects it from the palette); it's popped when the user
//! confirms a result, cancels, or submits an empty query. No
//! per-open state to reset because the object itself is discarded
//! and rebuilt.

pub mod panel;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::AppMsg;
use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Component, Registrar};
use crate::shared::async_job::AsyncJob;
use crate::shared::nominatim::{NominatimClient, SearchResult};

pub struct SearchComponent {
    pub(in crate::plugin::search) query: String,
    pub(in crate::plugin::search) candidates: Vec<SearchResult>,
    pub(in crate::plugin::search) selected: usize,
    client: Arc<NominatimClient>,
    job: AsyncJob<Vec<SearchResult>>,
}

impl SearchComponent {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
            client: nominatim,
            job: AsyncJob::new(),
        }
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }
}

impl Component for SearchComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(event.code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && event.code == KeyCode::Char('p'));
            let down = matches!(event.code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && event.code == KeyCode::Char('n'));

            if event.code == KeyCode::Esc {
                win.close();
            } else if event.code == KeyCode::Enter {
                let loc = self.candidates[self.selected].location;
                win.emit(AppMsg::Jump(loc));
                win.close();
            } else if up && self.selected > 0 {
                self.selected -= 1;
            } else if down && self.selected + 1 < self.candidates.len() {
                self.selected += 1;
            }
            return;
        }

        match event.code {
            KeyCode::Esc => win.close(),
            KeyCode::Enter => {
                if self.query.is_empty() {
                    win.close();
                } else {
                    let client = self.client.clone();
                    let query = self.query.clone();
                    self.job.spawn(move || client.search(&query));
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
            }
            KeyCode::Char(c) => self.query.push(c),
            // Modal: any other key is implicitly consumed (no
            // `win.ignore()`), so it doesn't fall through to keymap.
            _ => {}
        }
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(self, win);
    }

    fn poll(&mut self, _win: &mut Window) {
        if let Some(results) = self.job.poll() {
            self.candidates = results;
            self.selected = 0;
        }
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    }

    fn name(&self) -> &'static str {
        "search"
    }
}

/// Wire the search plugin into the registrar. Adds:
/// - activation on `/` → push a fresh [`SearchComponent`]
/// - palette entry so the picker can reach it
pub fn register(nominatim: Arc<NominatimClient>, r: &mut Registrar) {
    let nominatim_for_key = nominatim.clone();
    r.bind(KeyCode::Char('/'), KeyModifiers::NONE, move |_| {
        SearchComponent::new(nominatim_for_key.clone())
    });
    r.add_spawn("Search location", "/", move |_| {
        SearchComponent::new(nominatim.clone())
    });
}
