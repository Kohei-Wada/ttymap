//! [`CommandProvider`] — the default palette provider.
//!
//! Built once at startup from:
//! - map actions (harvested from [`MapAction::all_listed`] with keymap
//!   hints)
//! - plugin palette entries harvested from the
//!   [`Registrar`](crate::lua::Registrar) and drained into this
//!   provider at composition time
//!
//! A dynamic "Theme" entry is appended per-open with the current
//! `ThemeId` so the "(current)" hint stays accurate after runtime
//! theme changes.

use std::rc::Rc;

use crate::UserCommand;
use crate::compositor::{Context, PaletteEntry as RegistrarEntry};
use crate::input::keymap::KeyMap;
use crate::map::MapAction;
use crate::theme::ThemeId;

use super::{PaletteAction, PaletteItem, PaletteProvider, ThemeProvider};

/// Static snapshot of the default provider's entry list — built once
/// at composition time, held as `Rc` so the palette activation
/// closure can clone cheaply for each push.
pub struct CommandSeed {
    map_actions: Vec<(MapAction, String)>, // (action, key hint)
    plugin_entries: Vec<RegistrarEntry>,
}

impl CommandSeed {
    pub fn build(keymap: &KeyMap, plugin_entries: Vec<RegistrarEntry>) -> Self {
        let map_actions = MapAction::all_listed()
            .iter()
            .map(|a| {
                let hint = keymap.keys_for(&UserCommand::Map(a.clone())).join(", ");
                (a.clone(), hint)
            })
            .collect();
        Self {
            map_actions,
            plugin_entries,
        }
    }
}

#[derive(Clone)]
enum Kind {
    /// Plain map action — selecting runs `UserCommand::Map(action)`.
    MapAction(MapAction),
    /// Plugin-registered Spawn / Run entry — selecting invokes the
    /// closure in `CommandSeed::plugin_entries[idx]`.
    PluginEntry(usize),
    /// "Theme" entry — swaps provider to [`ThemeProvider`].
    OpenThemeProvider(ThemeId),
}

#[derive(Clone)]
struct Entry {
    label: String,
    hint: String,
    kind: Kind,
}

pub struct CommandProvider {
    seed: Rc<CommandSeed>,
    all: Vec<Entry>,
    /// Indices into `all` matching the current query.
    filtered: Vec<usize>,
    items: Vec<PaletteItem>,
}

impl CommandProvider {
    pub fn build(seed: Rc<CommandSeed>, current_theme: ThemeId) -> Self {
        let mut all: Vec<Entry> = Vec::new();

        for (action, hint) in &seed.map_actions {
            all.push(Entry {
                label: action.label().to_string(),
                hint: hint.clone(),
                kind: Kind::MapAction(action.clone()),
            });
        }

        for (i, entry) in seed.plugin_entries.iter().enumerate() {
            all.push(Entry {
                label: entry.label.clone(),
                hint: entry.hint.clone(),
                kind: Kind::PluginEntry(i),
            });
        }

        all.push(Entry {
            label: "Theme".to_string(),
            hint: current_theme.name().to_string(),
            kind: Kind::OpenThemeProvider(current_theme),
        });

        let mut provider = Self {
            seed,
            all,
            filtered: Vec::new(),
            items: Vec::new(),
        };
        provider.filter("");
        provider
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

    fn execute(&mut self, idx: usize, ctx: &Context) -> PaletteAction {
        let Some(&entry_idx) = self.filtered.get(idx) else {
            return PaletteAction::Close;
        };
        match &self.all[entry_idx].kind {
            Kind::MapAction(a) => PaletteAction::Run(vec![UserCommand::Map(a.clone())]),
            Kind::PluginEntry(i) => {
                let entry = &self.seed.plugin_entries[*i];
                // Factory may decline (Lua plugin returned falsy);
                // close the palette without pushing in that case.
                match (entry.spawn)(ctx) {
                    Some(c) => PaletteAction::Push(c),
                    None => PaletteAction::Close,
                }
            }
            Kind::OpenThemeProvider(current) => {
                PaletteAction::SwitchProvider(Box::new(ThemeProvider::new(*current)))
            }
        }
    }
}
