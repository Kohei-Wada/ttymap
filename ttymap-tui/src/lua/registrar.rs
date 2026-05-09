//! `PluginRegistry` — live home for plugin-declared activations and
//! palette entries.
//!
//! Each `.lua` file under any runtime layer's `plugin/` directory
//! registers via `register_keybind` / `register_palette_command`.
//! The captured [`KeybindSpec`](crate::lua::capture::KeybindSpec) /
//! [`PaletteCommandSpec`](crate::lua::capture::PaletteCommandSpec)
//! are converted to [`Activation`] / [`PaletteEntry`] by
//! [`crate::lua::loader::register_one`] and added to a single shared
//! [`PluginRegistry`] paired with the handle ID Lua received.
//!
//! The registry lives behind an `Rc<RefCell<...>>` so:
//!
//! - [`crate::compositor::BaseLayer`] borrows it on each keypress to
//!   dispatch
//! - The `:` palette installer borrows it on each open to build a
//!   fresh `CommandSeed` snapshot
//! - `PaletteCommandHandle:remove()` / `KeybindHandle:remove()` from
//!   Lua mutably borrow it to drop the matching entry by ID
//!
//! This module replaces the previous transient `Registrar` struct
//! whose fields were moved out into `BaseLayer` / palette installer
//! at startup — leaving Lua handles with nothing to remove from.
//! `PluginRegistry` stays alive for the program's lifetime, owned
//! jointly through `Rc` clones by every consumer.
//!
//! Lives in `lua/` rather than `compositor/` because it's purely the
//! Lua side's collection — the compositor itself never names it.

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::compositor::{Activation, PaletteEntry};

/// Live registry of plugin-supplied activations + palette entries.
/// Each entry is paired with the handle ID Lua holds, so a
/// `:remove()` from Lua can find and drop the exact entry.
///
/// Order is registration order (a `Vec<(id, T)>`), mirroring the
/// loader's iteration order over capture specs. Removal is `retain`
/// — `O(n)` linear, but the entry counts top out at the
/// total-plugin-count plus a handful per plugin in practice.
#[derive(Default)]
pub struct PluginRegistry {
    activations: Vec<(u64, Activation)>,
    palette_entries: Vec<(u64, PaletteEntry)>,
}

/// Cheap-clone shared owner of a [`PluginRegistry`]. Cloned into
/// every consumer that needs to read or mutate the registry.
pub type PluginRegistryHandle = Rc<RefCell<PluginRegistry>>;

pub fn new_plugin_registry() -> PluginRegistryHandle {
    Rc::new(RefCell::new(PluginRegistry::default()))
}

impl PluginRegistry {
    pub fn add_activation(&mut self, id: u64, a: Activation) {
        self.activations.push((id, a));
    }

    pub fn add_palette_entry(&mut self, id: u64, e: PaletteEntry) {
        self.palette_entries.push((id, e));
    }

    /// Drop the activation registered with `id`. Returns true if a
    /// matching entry was found.
    pub fn remove_activation(&mut self, id: u64) -> bool {
        let before = self.activations.len();
        self.activations.retain(|(i, _)| *i != id);
        before != self.activations.len()
    }

    /// Drop the palette entry registered with `id`. Returns true if
    /// a matching entry was found.
    pub fn remove_palette_entry(&mut self, id: u64) -> bool {
        let before = self.palette_entries.len();
        self.palette_entries.retain(|(i, _)| *i != id);
        before != self.palette_entries.len()
    }

    /// Find the first activation matching `(code, modifiers)`.
    /// Returns the [`Activation`] reference for the caller to invoke
    /// `.spawn` on directly while the borrow is still alive.
    pub fn find_activation(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&Activation> {
        self.activations.iter().find_map(|(_, a)| {
            if a.code == code && a.modifiers == modifiers {
                Some(a)
            } else {
                None
            }
        })
    }

    /// Iterate `(id, entry)` pairs in registration order. Used by
    /// the `:` palette installer when building a fresh `CommandSeed`
    /// snapshot per open.
    pub fn palette_entries(&self) -> &[(u64, PaletteEntry)] {
        &self.palette_entries
    }

    /// Look up a palette entry by its handle ID. Used by
    /// `CommandProvider::execute` so a stale snapshot pointing at a
    /// since-removed entry resolves to `None` instead of dispatching
    /// against a phantom factory.
    pub fn palette_entry(&self, id: u64) -> Option<&PaletteEntry> {
        self.palette_entries
            .iter()
            .find_map(|(i, e)| if *i == id { Some(e) } else { None })
    }

    pub fn palette_entry_count(&self) -> usize {
        self.palette_entries.len()
    }

    pub fn activation_count(&self) -> usize {
        self.activations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Component;

    fn fake_activation(c: char) -> Activation {
        Activation {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            spawn: std::rc::Rc::new(|_| -> Option<Box<dyn Component>> { None }),
        }
    }

    fn fake_palette_entry(label: &str) -> PaletteEntry {
        PaletteEntry {
            label: label.to_string(),
            hint: String::new(),
            name: "test",
            spawn: std::rc::Rc::new(|_| -> Option<Box<dyn Component>> { None }),
        }
    }

    #[test]
    fn add_then_remove_activation_round_trip() {
        let mut r = PluginRegistry::default();
        r.add_activation(7, fake_activation('a'));
        r.add_activation(8, fake_activation('b'));
        assert_eq!(r.activation_count(), 2);

        assert!(r.remove_activation(7));
        assert_eq!(r.activation_count(), 1);
        assert!(
            r.find_activation(KeyCode::Char('a'), KeyModifiers::NONE)
                .is_none()
        );
        assert!(
            r.find_activation(KeyCode::Char('b'), KeyModifiers::NONE)
                .is_some()
        );

        assert!(!r.remove_activation(7), "second remove returns false");
        assert!(!r.remove_activation(999), "missing id returns false");
    }

    #[test]
    fn add_then_remove_palette_entry_round_trip() {
        let mut r = PluginRegistry::default();
        r.add_palette_entry(11, fake_palette_entry("alpha"));
        r.add_palette_entry(12, fake_palette_entry("beta"));
        assert_eq!(r.palette_entry_count(), 2);

        assert!(r.remove_palette_entry(11));
        assert!(r.palette_entry(11).is_none());
        assert_eq!(r.palette_entry(12).map(|e| e.label.as_str()), Some("beta"));
    }

    #[test]
    fn palette_entries_iter_preserves_registration_order() {
        let mut r = PluginRegistry::default();
        r.add_palette_entry(1, fake_palette_entry("first"));
        r.add_palette_entry(2, fake_palette_entry("second"));
        r.add_palette_entry(3, fake_palette_entry("third"));
        let labels: Vec<&str> = r
            .palette_entries()
            .iter()
            .map(|(_, e)| e.label.as_str())
            .collect();
        assert_eq!(labels, vec!["first", "second", "third"]);
    }
}
