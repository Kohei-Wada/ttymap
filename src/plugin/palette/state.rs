//! Palette state — query buffer, filtered index list, selection.

use crossterm::event::{KeyCode, KeyModifiers};

use super::commands::Command;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Outcome {
    None,
    Consumed,
    Run(usize),
}

pub(super) struct PaletteState {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) commands: Vec<Command>,
    /// Indices into `commands` matching the current query, in display
    /// order. Rebuilt on every query edit.
    pub(super) filtered: Vec<usize>,
    pub(super) selected: usize,
}

impl PaletteState {
    pub(super) fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            commands: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn set_commands(&mut self, commands: Vec<Command>) {
        self.commands = commands;
    }

    pub(super) fn open(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.active = true;
        self.rebuild_filter();
    }

    pub(super) fn close(&mut self) {
        self.query.clear();
        self.filtered.clear();
        self.selected = 0;
        self.active = false;
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
                if let Some(&idx) = self.filtered.get(self.selected) {
                    self.active = false;
                    Outcome::Run(idx)
                } else {
                    self.active = false;
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
                if self.selected + 1 < self.filtered.len() {
                    self.selected += 1;
                }
                Outcome::Consumed
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.rebuild_filter();
                Outcome::Consumed
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.rebuild_filter();
                Outcome::Consumed
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.rebuild_filter();
                Outcome::Consumed
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.rebuild_filter();
                Outcome::Consumed
            }
            _ => Outcome::None,
        }
    }

    fn rebuild_filter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = if q.is_empty() {
            (0..self.commands.len()).collect()
        } else {
            let mut ranked: Vec<(usize, usize)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let label = c.label.to_lowercase();
                    label.find(&q).map(|pos| (pos, i))
                })
                .collect();
            // Earlier match position first; break ties by registration order.
            ranked.sort_by_key(|&(pos, i)| (pos, i));
            ranked.into_iter().map(|(_, i)| i).collect()
        };
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Action;
    use crate::plugin::palette::commands::CommandKind;

    fn cmd(label: &str) -> Command {
        Command {
            label: label.to_string(),
            keys: String::new(),
            kind: CommandKind::Action(Action::None),
        }
    }

    fn state_with(labels: &[&str]) -> PaletteState {
        let mut s = PaletteState::new();
        s.set_commands(labels.iter().map(|l| cmd(l)).collect());
        s.open();
        s
    }

    #[test]
    fn filter_empty_query_lists_all() {
        let s = state_with(&["Zoom in", "Zoom out", "Quit"]);
        assert_eq!(s.filtered, vec![0, 1, 2]);
    }

    #[test]
    fn filter_substring_case_insensitive() {
        let mut s = state_with(&["Zoom in", "Zoom out", "Quit"]);
        s.handle_key(KeyCode::Char('Z'), KeyModifiers::NONE);
        assert_eq!(s.filtered, vec![0, 1]);
    }

    #[test]
    fn filter_earlier_match_ranks_first() {
        let mut s = state_with(&["Zoom in", "Quit"]);
        // 'i' appears at pos 2 of "Quit" and pos 5 of "Zoom in" →
        // "Quit" (index 1) ranks first.
        s.handle_key(KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(s.filtered, vec![1, 0]);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut s = state_with(&["Zoom in", "Quit"]);
        s.handle_key(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(s.filtered, vec![0]);
        s.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(s.filtered, vec![0, 1]);
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
        assert!(s.filtered.is_empty());
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
