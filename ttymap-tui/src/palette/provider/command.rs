//! [`CommandProvider`] — the default palette provider.
//!
//! Built per palette open from:
//! - map actions (harvested from [`MapAction::all_listed`] with keymap
//!   hints — static, cached on [`CommandSeed`])
//! - plugin palette entries — read live from the shared
//!   [`LuaRegistry`](crate::lua::LuaRegistry) so plugin
//!   `:remove()` / dynamic registration is reflected on the next
//!   palette open
//!
//! A dynamic "Theme" entry is appended per-open with the current
//! `ThemeId` so the "(current)" hint stays accurate after runtime
//! theme changes.
//!
//! Plugin entries carry their handle ID (`Kind::PluginEntry(u64)`)
//! rather than a `Vec` index, so a stale snapshot pointing at a
//! since-removed entry resolves to `None` at execute time and
//! silently no-ops instead of dispatching against a phantom factory.

use std::rc::Rc;

use crate::UserCommand;
use crate::compositor::Context;
use crate::input::keymap::KeyMap;
use crate::lua::LuaRegistryHandle;
use crate::theme::ThemeId;
use ttymap_engine::map::MapAction;

use super::{PaletteAction, PaletteItem, PaletteProvider, ThemeProvider};

/// Cached static-half of the palette source. Held as `Rc` so the
/// `:` activation closure can clone cheaply for each push. The
/// dynamic plugin half is read from `registry` at
/// [`CommandProvider::build`] time per open.
pub struct CommandSeed {
    map_actions: Vec<(MapAction, String)>, // (action, key hint)
    registry: LuaRegistryHandle,
}

impl CommandSeed {
    pub fn build(keymap: &KeyMap, registry: LuaRegistryHandle) -> Self {
        let map_actions = MapAction::all_listed()
            .iter()
            .map(|a| {
                let hint = keymap.keys_for(&UserCommand::Map(a.clone())).join(", ");
                (a.clone(), hint)
            })
            .collect();
        Self {
            map_actions,
            registry,
        }
    }
}

#[derive(Clone)]
enum Kind {
    /// Plain map action — selecting runs `UserCommand::Map(action)`.
    MapAction(MapAction),
    /// Plugin-registered entry — selecting looks up the matching
    /// [`PaletteEntry`](crate::compositor::PaletteEntry) in the live
    /// [`LuaRegistry`](crate::lua::LuaRegistry) by ID and
    /// invokes its factory. If the entry has been `:remove()`d
    /// since this snapshot was built, the lookup returns `None` and
    /// the palette closes silently.
    PluginEntry(u64),
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

        // Snapshot the plugin entries from the live registry. We
        // keep label / hint locally so the palette can render
        // without re-borrowing during paint; the factory itself
        // stays in the registry and we look it up by id at execute
        // time.
        for (id, entry) in seed.registry.borrow().palette_entries() {
            all.push(Entry {
                label: entry.label.clone(),
                hint: entry.hint.clone(),
                kind: Kind::PluginEntry(*id),
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
            Kind::PluginEntry(id) => {
                // Live lookup against the registry — the entry may
                // have been `:remove()`d between palette open and
                // selection. Clone the factory out under a short
                // borrow and drop the borrow before invoking, so
                // the spawn callback is free to mutably borrow the
                // registry (e.g. to call `:remove()` on its own
                // handle).
                let factory = self
                    .seed
                    .registry
                    .borrow()
                    .palette_entry(*id)
                    .map(|e| e.spawn.clone());
                let Some(spawn) = factory else {
                    log::info!("palette: entry id={} no longer registered, ignoring", id);
                    return PaletteAction::Close;
                };
                // Factory may decline (Lua plugin returned falsy);
                // close the palette without pushing in that case.
                match spawn(ctx) {
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
