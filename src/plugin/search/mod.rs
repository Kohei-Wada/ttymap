//! Search widget — center popup for forward geocoding.
//!
//! Under the compositor model, search is an **ephemeral** component:
//! a fresh instance is pushed onto the stack when the user hits `/`
//! (or selects it from the palette); it's popped when the user
//! confirms a result, cancels, or submits an empty query. No
//! per-open state to reset because the object itself is discarded
//! and rebuilt.

pub mod panel;
mod service;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::compositor::{
    Activation, Component, Context, EventResult, PaletteEntry, PaletteKind, Registrar,
};
use crate::shared::nominatim::{NominatimClient, SearchResult};
use crate::theme::UiTheme;

use service::SearchService;

pub struct SearchComponent {
    pub(in crate::plugin::search) query: String,
    pub(in crate::plugin::search) candidates: Vec<SearchResult>,
    pub(in crate::plugin::search) selected: usize,
    service: SearchService,
}

impl SearchComponent {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
            service: SearchService::new(nominatim),
        }
    }

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }
}

impl Component for SearchComponent {
    fn handle_event(&mut self, event: KeyEvent, _ctx: &Context) -> EventResult {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        // Let Tab/Shift-Tab bubble to the base layer so focus cycle
        // works even when the search popup is the top modal.
        if matches!(event.code, KeyCode::Tab | KeyCode::BackTab) {
            return EventResult::Ignored;
        }

        if self.has_candidates() {
            let up = matches!(event.code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && event.code == KeyCode::Char('p'));
            let down = matches!(event.code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && event.code == KeyCode::Char('n'));

            return match event.code {
                KeyCode::Esc => EventResult::Close(Vec::new()),
                KeyCode::Enter => {
                    let loc = self.candidates[self.selected].location;
                    EventResult::Close(vec![AppMsg::Jump(loc)])
                }
                _ if up => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                    EventResult::Consumed(Vec::new())
                }
                _ if down => {
                    if self.selected + 1 < self.candidates.len() {
                        self.selected += 1;
                    }
                    EventResult::Consumed(Vec::new())
                }
                _ => EventResult::Consumed(Vec::new()),
            };
        }

        match event.code {
            KeyCode::Esc => EventResult::Close(Vec::new()),
            KeyCode::Enter => {
                if self.query.is_empty() {
                    EventResult::Close(Vec::new())
                } else {
                    self.service.search(&self.query);
                    EventResult::Consumed(Vec::new())
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                EventResult::Consumed(Vec::new())
            }
            // Modal: any other key is still consumed (don't fall
            // through to the keymap while the search popup is up).
            _ => EventResult::Consumed(Vec::new()),
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    fn poll(&mut self) -> Vec<AppMsg> {
        if let Some(results) = self.service.poll() {
            self.candidates = results;
            self.selected = 0;
        }
        Vec::new()
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    }
}

/// Wire the search plugin into the registrar. Adds:
/// - activation on `/` → push a fresh [`SearchComponent`]
/// - palette entry so the picker can reach it
pub fn register(nominatim: Arc<NominatimClient>, r: &mut Registrar) {
    let spawn_for_activation = {
        let nominatim = nominatim.clone();
        move |_ctx: &Context| -> Box<dyn Component> {
            Box::new(SearchComponent::new(nominatim.clone()))
        }
    };
    r.add_activation(Activation {
        code: KeyCode::Char('/'),
        modifiers: KeyModifiers::NONE,
        spawn: Box::new(spawn_for_activation),
    });

    let spawn_for_palette = move |_ctx: &Context| -> Box<dyn Component> {
        Box::new(SearchComponent::new(nominatim.clone()))
    };
    r.add_palette_entry(PaletteEntry {
        label: "Search location".to_string(),
        hint: "/".to_string(),
        kind: PaletteKind::Spawn(Box::new(spawn_for_palette)),
    });
}
