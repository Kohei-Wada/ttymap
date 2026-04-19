//! Palette state — query buffer, selection, key handling.
//!
//! The actual item list and filter logic live on the current
//! [`PaletteProvider`] — `state` just routes queries to it and tracks
//! selection within `provider.items()`.

use crossterm::event::{KeyCode, KeyModifiers};

use super::provider::PaletteProvider;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Outcome {
    None,
    Consumed,
    /// User pressed Enter on `provider.items()[idx]`.
    Run(usize),
}

pub(super) struct PaletteState {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) selected: usize,
    pub(super) provider: Option<Box<dyn PaletteProvider>>,
}

impl PaletteState {
    pub(super) fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            selected: 0,
            provider: None,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn open_with(&mut self, provider: Box<dyn PaletteProvider>) {
        self.query.clear();
        self.selected = 0;
        self.provider = Some(provider);
        self.active = true;
        if let Some(p) = self.provider.as_mut() {
            p.filter("");
        }
    }

    pub(super) fn items_len(&self) -> usize {
        self.provider.as_ref().map(|p| p.items().len()).unwrap_or(0)
    }

    pub(super) fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Outcome {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        let up = matches!(code, KeyCode::Up) || (ctrl && code == KeyCode::Char('p'));
        let down = matches!(code, KeyCode::Down) || (ctrl && code == KeyCode::Char('n'));

        match code {
            KeyCode::Esc => {
                self.active = false;
                Outcome::Consumed
            }
            KeyCode::Enter => {
                let has_item = self.selected < self.items_len();
                self.active = false;
                if has_item {
                    Outcome::Run(self.selected)
                } else {
                    Outcome::Consumed
                }
            }
            _ if up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Outcome::Consumed
            }
            _ if down => {
                if self.selected + 1 < self.items_len() {
                    self.selected += 1;
                }
                Outcome::Consumed
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
                Outcome::Consumed
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.refilter();
                Outcome::Consumed
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
                Outcome::Consumed
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refilter();
                Outcome::Consumed
            }
            _ => Outcome::None,
        }
    }

    fn refilter(&mut self) {
        if let Some(p) = self.provider.as_mut() {
            p.filter(&self.query);
        }
        let n = self.items_len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::map::Action;
    use crate::ui::palette::provider::{PaletteAction, PaletteItem, PaletteProvider};

    /// Minimal provider that just lists the labels we give it, with a
    /// simple substring filter matching the real `CommandProvider`.
    struct FakeProvider {
        all: Vec<String>,
        filtered: Vec<usize>,
        items: Vec<PaletteItem>,
    }

    impl FakeProvider {
        fn new(labels: &[&str]) -> Self {
            let mut p = Self {
                all: labels.iter().map(|s| s.to_string()).collect(),
                filtered: Vec::new(),
                items: Vec::new(),
            };
            p.filter("");
            p
        }
    }

    impl PaletteProvider for FakeProvider {
        fn prompt(&self) -> &str {
            ":"
        }
        fn filter(&mut self, query: &str) {
            let q = query.to_lowercase();
            self.filtered = if q.is_empty() {
                (0..self.all.len()).collect()
            } else {
                let mut ranked: Vec<(usize, usize)> = self
                    .all
                    .iter()
                    .enumerate()
                    .filter_map(|(i, l)| l.to_lowercase().find(&q).map(|pos| (pos, i)))
                    .collect();
                ranked.sort_by_key(|&(pos, i)| (pos, i));
                ranked.into_iter().map(|(_, i)| i).collect()
            };
            self.items = self
                .filtered
                .iter()
                .map(|&i| PaletteItem {
                    label: self.all[i].clone(),
                    hint: String::new(),
                })
                .collect();
        }
        fn items(&self) -> &[PaletteItem] {
            &self.items
        }
        fn execute(&mut self, _idx: usize) -> PaletteAction {
            PaletteAction::Run(Command::Map(Action::None))
        }
    }

    fn state_with(labels: &[&str]) -> PaletteState {
        let mut s = PaletteState::new();
        s.open_with(Box::new(FakeProvider::new(labels)));
        s
    }

    fn filtered_labels(s: &PaletteState) -> Vec<&str> {
        s.provider
            .as_ref()
            .unwrap()
            .items()
            .iter()
            .map(|i| i.label.as_str())
            .collect()
    }

    #[test]
    fn filter_empty_query_lists_all() {
        let s = state_with(&["Zoom in", "Zoom out", "Quit"]);
        assert_eq!(filtered_labels(&s), vec!["Zoom in", "Zoom out", "Quit"]);
    }

    #[test]
    fn filter_substring_case_insensitive() {
        let mut s = state_with(&["Zoom in", "Zoom out", "Quit"]);
        s.handle_key(KeyCode::Char('Z'), KeyModifiers::NONE);
        assert_eq!(filtered_labels(&s), vec!["Zoom in", "Zoom out"]);
    }

    #[test]
    fn filter_earlier_match_ranks_first() {
        let mut s = state_with(&["Zoom in", "Quit"]);
        // 'i' at pos 2 of "Quit" vs pos 5 of "Zoom in" → Quit first.
        s.handle_key(KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(filtered_labels(&s), vec!["Quit", "Zoom in"]);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut s = state_with(&["Zoom in", "Quit"]);
        s.handle_key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(filtered_labels(&s), vec!["Zoom in"]);
        s.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(filtered_labels(&s), vec!["Zoom in", "Quit"]);
    }

    #[test]
    fn down_up_stays_in_bounds() {
        let mut s = state_with(&["A", "B", "C"]);
        s.handle_key(KeyCode::Down, KeyModifiers::NONE);
        s.handle_key(KeyCode::Down, KeyModifiers::NONE);
        s.handle_key(KeyCode::Down, KeyModifiers::NONE); // past end
        assert_eq!(s.selected, 2);
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        s.handle_key(KeyCode::Up, KeyModifiers::NONE);
        s.handle_key(KeyCode::Up, KeyModifiers::NONE); // past top
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn enter_returns_run_with_selected_index() {
        let mut s = state_with(&["A", "B", "C"]);
        s.handle_key(KeyCode::Down, KeyModifiers::NONE);
        let outcome = s.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(outcome, Outcome::Run(1));
        assert!(!s.is_active());
    }

    #[test]
    fn enter_with_empty_filter_closes_without_run() {
        let mut s = state_with(&["Zoom in"]);
        s.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(filtered_labels(&s).is_empty());
        let outcome = s.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(outcome, Outcome::Consumed);
        assert!(!s.is_active());
    }

    #[test]
    fn esc_closes() {
        let mut s = state_with(&["A"]);
        s.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!s.is_active());
    }

    #[test]
    fn ctrl_u_clears_query() {
        let mut s = state_with(&["A"]);
        s.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('b'), KeyModifiers::NONE);
        s.handle_key(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(s.query, "");
    }
}
