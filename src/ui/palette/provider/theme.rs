//! [`ThemeProvider`] — palette sub-mode for runtime theme switching.
//!
//! Reached from the default command provider by selecting the "Theme"
//! entry (or, in the future, a `:theme` shortcut). Lists every
//! registered [`ThemeId`] with the current one marked.

use crate::color_palette::ThemeId;
use crate::app_msg::AppMsg;
use crate::ui::action::UiAction;

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

    fn execute(&mut self, idx: usize) -> PaletteAction {
        match self.filtered_theme_ids.get(idx) {
            Some(&t) => PaletteAction::Run(AppMsg::Ui(UiAction::SetTheme(t))),
            None => PaletteAction::Close,
        }
    }
}
