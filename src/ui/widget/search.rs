//! Search widget — input, completion, candidate selection, and overlay rendering.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, List, ListItem, Paragraph};

use crate::nominatim::SearchResult;
use crate::ui::theme;

#[derive(Debug, Clone, PartialEq)]
pub enum SearchAction {
    None,
    /// Query text changed — app should trigger completion after debounce.
    QueryChanged(String),
    /// User pressed Enter — execute full search.
    Submit(String),
    /// User selected a candidate from results.
    Select(usize),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// Typing query, completions shown below.
    Input,
    /// Browsing search results after Enter.
    Results,
}

pub struct SearchWidget {
    query: String,
    active: bool,
    mode: Mode,
    completions: Vec<SearchResult>,
    completion_selected: usize,
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
            mode: Mode::Input,
            completions: Vec::new(),
            completion_selected: 0,
            candidates: Vec::new(),
            selected: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn open(&mut self) {
        self.query.clear();
        self.completions.clear();
        self.completion_selected = 0;
        self.candidates.clear();
        self.selected = 0;
        self.mode = Mode::Input;
        self.active = true;
    }

    /// Set completion suggestions (from debounced autocomplete).
    pub fn set_completions(&mut self, completions: Vec<SearchResult>) {
        self.completions = completions;
        self.completion_selected = 0;
    }

    /// Set full search results (from Enter).
    pub fn set_candidates(&mut self, candidates: Vec<SearchResult>) {
        self.candidates = candidates;
        self.selected = 0;
        self.mode = Mode::Results;
    }

    pub fn has_candidates(&self) -> bool {
        match self.mode {
            Mode::Results => !self.candidates.is_empty(),
            Mode::Input => false,
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> SearchAction {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        // Results mode: browsing search results
        if self.mode == Mode::Results && !self.candidates.is_empty() {
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

        // Input mode
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
                    self.completions.clear();
                    SearchAction::Submit(self.query.clone())
                }
            }
            KeyCode::Tab => {
                // Accept selected completion
                if !self.completions.is_empty()
                    && let Some(c) = self.completions.get(self.completion_selected)
                {
                    self.query = c.name.clone();
                    self.completions.clear();
                }
                SearchAction::None
            }
            KeyCode::Down | KeyCode::Char('n') if ctrl || code == KeyCode::Down => {
                if !self.completions.is_empty()
                    && self.completion_selected + 1 < self.completions.len()
                {
                    self.completion_selected += 1;
                }
                SearchAction::None
            }
            KeyCode::Up | KeyCode::Char('p') if ctrl || code == KeyCode::Up => {
                if self.completion_selected > 0 {
                    self.completion_selected -= 1;
                }
                SearchAction::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.query_changed()
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.query_changed()
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.completions.clear();
                SearchAction::None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.query_changed()
            }
            _ => SearchAction::None,
        }
    }

    fn query_changed(&mut self) -> SearchAction {
        self.completions.clear();
        self.completion_selected = 0;
        if self.query.len() >= 3 {
            SearchAction::QueryChanged(self.query.clone())
        } else {
            SearchAction::None
        }
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect) {
        if !self.active || map_inner.width < 10 || map_inner.height < 3 {
            return;
        }

        let popup_width = (map_inner.width * 2 / 3).max(30).min(map_inner.width - 2);

        let list_len = match self.mode {
            Mode::Results => self.candidates.len(),
            Mode::Input => self.completions.len(),
        };
        let popup_height = if list_len > 0 {
            (list_len as u16 + 4).min(map_inner.height - 2)
        } else {
            3
        };

        let x = map_inner.x + (map_inner.width - popup_width) / 2;
        let y = map_inner.y + 1;

        let popup_area = Rect::new(x, y, popup_width, popup_height);
        f.render_widget(Clear, popup_area);

        match self.mode {
            Mode::Results if !self.candidates.is_empty() => {
                self.render_candidates(f, popup_area);
            }
            _ if !self.completions.is_empty() => {
                self.render_with_completions(f, popup_area);
            }
            _ => {
                self.render_input(f, popup_area);
            }
        }
    }

    fn render_input(&self, f: &mut Frame, area: Rect) {
        let block = theme::panel("search");
        let widget = Paragraph::new(format!("/{}", self.query))
            .style(theme::text())
            .block(block);
        f.render_widget(widget, area);
    }

    fn render_with_completions(&self, f: &mut Frame, area: Rect) {
        let block = theme::panel("search");

        let mut items: Vec<ListItem> = Vec::new();
        // First line: query input
        items.push(ListItem::new(format!("/{}", self.query)).style(theme::text()));

        // Completion suggestions
        for (i, result) in self.completions.iter().enumerate() {
            let style = if i == self.completion_selected {
                theme::selected()
            } else {
                theme::muted()
            };
            items.push(ListItem::new(format!("  {}", result.name)).style(style));
        }

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }

    fn render_candidates(&self, f: &mut Frame, area: Rect) {
        let title = format!("search: {}", self.query);
        let block = theme::panel(&title);

        let items: Vec<ListItem> = self
            .candidates
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let style = if i == self.selected {
                    theme::selected()
                } else {
                    theme::text()
                };
                let prefix = if i == self.selected { "> " } else { "  " };
                ListItem::new(format!("{}{}", prefix, result.name)).style(style)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}
