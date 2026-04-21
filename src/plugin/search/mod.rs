//! Search widget — center popup for forward geocoding.
//!
//! Self-contained: owns its UI state, HTTP wrapper, and key dispatch.
//! `app.rs` sees it only through the [`Plugin`](super::Plugin) trait.

pub mod panel;
mod service;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app_command::{AppCommand, Effect, FocusSurface, SurfaceCtx};
use crate::shared::nominatim::{NominatimClient, SearchResult};
use crate::theme::UiTheme;

use service::SearchService;

use super::Plugin;

pub struct SearchPlugin {
    pub(in crate::plugin::search) query: String,
    pub(in crate::plugin::search) active: bool,
    pub(in crate::plugin::search) candidates: Vec<SearchResult>,
    pub(in crate::plugin::search) selected: usize,
    service: SearchService,
}

impl SearchPlugin {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            query: String::new(),
            active: false,
            candidates: Vec::new(),
            selected: 0,
            service: SearchService::new(nominatim),
        }
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }

    fn open(&mut self) {
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
        self.active = true;
    }

    fn close(&mut self) {
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
        self.active = false;
    }
}

impl Plugin for SearchPlugin {
    fn tag(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search location"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["/"]
    }

    fn activate(&mut self, _ctx: SurfaceCtx) {
        self.open();
    }

    fn deactivate(&mut self) {
        self.close();
    }

    fn poll(&mut self) -> bool {
        if let Some(results) = self.service.poll() {
            self.candidates = results;
            self.selected = 0;
            true
        } else {
            false
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }
}

/// Modal key dispatch. While candidates are showing the popup is in
/// "results mode" (Up/Down/Enter pick a hit, Esc cancels); otherwise
/// it's in "input mode" (typing edits the query, Enter submits a
/// forward-geocode). Focus release is host-driven — `ui::router`
/// notices `is_visible()=false` and releases for us.
impl FocusSurface for SearchPlugin {
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers, _ctx: SurfaceCtx) -> Effect {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && code == KeyCode::Char('p'));
            let down = matches!(code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && code == KeyCode::Char('n'));

            return if code == KeyCode::Esc {
                self.active = false;
                self.candidates.clear();
                Effect::Consumed
            } else if code == KeyCode::Enter {
                self.active = false;
                let loc = self.candidates[self.selected].location;
                self.candidates.clear();
                Effect::Run(AppCommand::Jump(loc))
            } else if up {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Effect::Consumed
            } else if down {
                if self.selected + 1 < self.candidates.len() {
                    self.selected += 1;
                }
                Effect::Consumed
            } else {
                Effect::Consumed
            };
        }

        match code {
            KeyCode::Esc => {
                self.active = false;
                Effect::Consumed
            }
            KeyCode::Enter => {
                if self.query.is_empty() {
                    self.active = false;
                } else {
                    self.service.search(&self.query);
                }
                Effect::Consumed
            }
            KeyCode::Backspace => {
                self.query.pop();
                Effect::Consumed
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                Effect::Consumed
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                Effect::Consumed
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                Effect::Consumed
            }
            // Modal: any other key is consumed (don't fall through to
            // the background while the search popup is up).
            _ => Effect::Consumed,
        }
    }

    fn is_visible(&self) -> bool {
        self.active
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    }
}
