//! [`CommandProvider`] — the default palette provider.
//!
//! Lists every map-level `Action` and every visible-to-the-user plugin
//! activation, filterable by substring. The palette shows this when
//! opened via `:`. Other providers (theme, search, wiki, …) will be
//! swapped in as the palette grows.

use crate::app::AppMsg;
use crate::color_palette::ThemeId;
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::plugin::Plugin;

use super::{PaletteAction, PaletteItem, PaletteProvider, ThemeProvider};

/// Internal entry — a command label + what happens when it runs.
#[derive(Clone)]
struct Entry {
    label: String,
    hint: String,
    kind: Kind,
}

#[derive(Clone)]
enum Kind {
    Action(Action),
    Activate(String),
    /// Opens the [`ThemeProvider`] sub-mode. Remembers the theme the
    /// palette was opened with so it can show "(current)" marker.
    OpenThemeProvider(ThemeId),
}

/// Snapshot of the static (theme-independent) portion of the command
/// provider's entry list — captured once at startup by
/// [`CommandProvider::snapshot`]. The dynamic "Theme" entry is
/// appended per-open by [`CommandProvider::build`] using the current
/// theme id.
///
/// Owned by `CommandPalette` so it can rebuild a fresh provider on
/// each activation without re-walking the plugin registry / keymap
/// (both of which are immutable post-startup).
#[derive(Clone, Default)]
pub struct CommandProviderSeed {
    static_entries: Vec<Entry>,
}

pub struct CommandProvider {
    all: Vec<Entry>,
    /// Indices into `all` matching the current query, in display order.
    filtered: Vec<usize>,
    /// Cached [`PaletteItem`] view of `filtered` — rebuilt by `filter`.
    items: Vec<PaletteItem>,
}

impl CommandProvider {
    /// Walk plugins + keymap once and capture the static portion of
    /// the entry list (map actions + each plugin's activation entry).
    /// The theme entry is *not* included — it depends on runtime
    /// state, so it's appended in [`Self::build`].
    pub fn snapshot(plugins: &[&dyn Plugin], keymap: &KeyMap) -> CommandProviderSeed {
        let mut entries: Vec<Entry> = Vec::new();

        for action in Action::all_listed() {
            entries.push(Entry {
                label: action.label().to_string(),
                hint: keymap.keys_for(&AppMsg::Map(action.clone())).join(", "),
                kind: Kind::Action(action.clone()),
            });
        }

        for p in plugins {
            let description = p.description();
            if description.is_empty() {
                continue;
            }
            entries.push(Entry {
                label: description.to_string(),
                hint: p.activation_keys().join(", "),
                kind: Kind::Activate(p.tag().to_string()),
            });
        }

        CommandProviderSeed {
            static_entries: entries,
        }
    }

    /// Build a fresh provider for one palette open. Combines the
    /// startup snapshot with the current theme (which seeds the
    /// "Theme" sub-mode entry's "(current)" hint).
    pub fn build(seed: &CommandProviderSeed, current_theme: ThemeId) -> Self {
        let mut all = seed.static_entries.clone();
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
            Kind::Action(a) => PaletteAction::Run(vec![AppMsg::Map(a.clone())]),
            Kind::Activate(tag) => PaletteAction::Open(tag.clone().into()),
            Kind::OpenThemeProvider(current) => {
                PaletteAction::SwitchProvider(Box::new(ThemeProvider::new(*current)))
            }
        }
    }
}
