//! [`ThemeProvider`] — palette sub-mode for runtime theme switching.
//!
//! Reached from [`CommandProvider`](super::CommandProvider) via the
//! "Theme" entry.

use crate::UserCommand;
use crate::core::compositor::Context;
use crate::theme::ThemeId;

use super::{PaletteAction, PaletteItem, PaletteProvider};

pub struct ThemeProvider {
    current: ThemeId,
    items: Vec<PaletteItem>,
    filtered_theme_ids: Vec<ThemeId>,
}

impl ThemeProvider {
    pub fn new(current: ThemeId) -> Self {
        let mut p = Self {
            current,
            items: Vec::new(),
            filtered_theme_ids: Vec::new(),
        };
        p.filter("");
        p
    }
}

impl PaletteProvider for ThemeProvider {
    fn prompt(&self) -> &str {
        "theme> "
    }

    fn filter(&mut self, query: &str) {
        let q = query.to_lowercase();
        self.filtered_theme_ids.clear();
        self.items.clear();
        for theme in ThemeId::all() {
            let name = theme.name();
            if !q.is_empty() && !name.contains(&q) {
                continue;
            }
            let hint = if *theme == self.current {
                "(current)".to_string()
            } else {
                String::new()
            };
            self.filtered_theme_ids.push(*theme);
            self.items.push(PaletteItem {
                label: name.to_string(),
                hint,
            });
        }
    }

    fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    fn execute(&mut self, idx: usize, _ctx: &Context) -> PaletteAction {
        match self.filtered_theme_ids.get(idx) {
            Some(&t) => PaletteAction::Run(vec![UserCommand::SetTheme(t)]),
            None => PaletteAction::Close,
        }
    }
}
