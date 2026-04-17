//! Search widget — input, candidate selection, and overlay rendering.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, List, ListItem, Paragraph};

use crate::nominatim::SearchResult;
use crate::ui::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum SearchAction {
    None,
    Submit(String),
    Select(usize),
    Cancel,
}

pub struct SearchWidget {
    query: String,
    active: bool,
    candidates: Vec<SearchResult>,
    selected: usize,
}

impl Default for SearchWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchWidget {
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

    pub fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
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

    pub fn render(&self, f: &mut Frame, map_inner: Rect, theme: &Theme) {
        if !self.active || map_inner.width < 10 || map_inner.height < 3 {
            return;
        }

        let popup_width = (map_inner.width * 2 / 3).max(30).min(map_inner.width - 2);
        let popup_height = if self.has_candidates() {
            (self.candidates.len() as u16 + 4).min(map_inner.height - 2)
        } else {
            3
        };

        let x = map_inner.x + (map_inner.width - popup_width) / 2;
        let y = map_inner.y + 1;

        let popup_area = Rect::new(x, y, popup_width, popup_height);
        f.render_widget(Clear, popup_area);

        if self.has_candidates() {
            self.render_candidates(f, popup_area, theme);
        } else {
            self.render_input(f, popup_area, theme);
        }
    }

    fn render_input(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = theme.panel("search");
        let widget = Paragraph::new(format!("/{}", self.query))
            .style(theme.text())
            .block(block);
        f.render_widget(widget, area);
    }

    fn render_candidates(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = format!("search: {}", self.query);
        let block = theme.panel(&title);

        let items: Vec<ListItem> = self
            .candidates
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let style = if i == self.selected {
                    theme.selected()
                } else {
                    theme.text()
                };
                let prefix = if i == self.selected { "> " } else { "  " };
                ListItem::new(format!("{}{}", prefix, result.name)).style(style)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}
