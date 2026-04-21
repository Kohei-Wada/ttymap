//! [`CommandProvider`] — the default palette provider.
//!
//! Lists every map-level `Action` and every visible-to-the-user plugin
//! activation, filterable by substring. The palette shows this when
//! opened via `:`. Other providers (theme, search, wiki, …) will be
//! swapped in as the palette grows.

use crate::app_command::AppCommand;
use crate::color_palette::ThemeId;
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::plugin::PluginRegistry;

use super::{PaletteAction, PaletteItem, PaletteProvider, ThemeProvider};

/// Internal entry — a command label + what happens when it runs.
struct Entry {
    label: String,
    hint: String,
    kind: Kind,
}

enum Kind {
    Action(Action),
    Activate(String),
    /// Opens the [`ThemeProvider`] sub-mode. Remembers the theme the
    /// palette was opened with so it can show "(current)" marker.
    OpenThemeProvider(ThemeId),
}

pub struct CommandProvider {
    all: Vec<Entry>,
    /// Indices into `all` matching the current query, in display order.
    filtered: Vec<usize>,
    /// Cached [`PaletteItem`] view of `filtered` — rebuilt by `filter`.
    items: Vec<PaletteItem>,
}

impl CommandProvider {
    pub fn new(widgets: &PluginRegistry, keymap: &KeyMap, current_theme: ThemeId) -> Self {
        let mut all: Vec<Entry> = Vec::new();

        for action in Action::all_listed() {
            all.push(Entry {
                label: action.label().to_string(),
                hint: keymap.keys_for(&AppCommand::Map(action.clone())).join(", "),
                kind: Kind::Action(action.clone()),
            });
        }

        for p in widgets.iter() {
            let description = p.description();
            if description.is_empty() {
                continue;
            }
            all.push(Entry {
                label: description.to_string(),
                hint: p.activation_keys().join(", "),
                kind: Kind::Activate(p.tag().to_string()),
            });
        }

        // Single entry that pivots into the theme sub-mode.
        all.push(Entry {
            label: "Theme".to_string(),
            hint: current_theme.name().to_string(),
            kind: Kind::OpenThemeProvider(current_theme),
        });

        let mut prov = Self {
            all,
            filtered: Vec::new(),
            items: Vec::new(),
        };
        prov.filter("");
        prov
    }
}

impl PaletteProvider for CommandProvider {
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
                .filter_map(|(i, e)| {
                    let label = e.label.to_lowercase();
                    label.find(&q).map(|pos| (pos, i))
                })
                .collect();
            // Earlier match position first; break ties by registration order.
            ranked.sort_by_key(|&(pos, i)| (pos, i));
            ranked.into_iter().map(|(_, i)| i).collect()
        };
        self.items = self
            .filtered
            .iter()
            .map(|&i| PaletteItem {
                label: self.all[i].label.clone(),
                hint: self.all[i].hint.clone(),
            })
            .collect();
    }

    fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    fn execute(&mut self, idx: usize) -> PaletteAction {
        let Some(&entry_idx) = self.filtered.get(idx) else {
            return PaletteAction::Close;
        };
        match &self.all[entry_idx].kind {
            Kind::Action(a) => PaletteAction::Run(AppCommand::Map(a.clone())),
            Kind::Activate(tag) => PaletteAction::Run(AppCommand::ActivatePlugin(tag.clone())),
            Kind::OpenThemeProvider(current) => {
                PaletteAction::SwitchProvider(Box::new(ThemeProvider::new(*current)))
            }
        }
    }
}
