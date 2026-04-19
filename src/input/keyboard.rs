//! Keyboard input handler. Pure **translator**: raw key event →
//! `Option<Command>`. The caller (`app.rs`) is the one that actually
//! dispatches the command, keeping keyboard / mouse / async-plugin
//! paths symmetric.
//!
//! The routing decision tree:
//!
//! 1. **Focus-first** — `command::deliver_key_to_focused` hands the
//!    event to the currently focused surface (palette / plugin). If it
//!    consumes or emits a `Command`, we're done.
//! 2. **Tab / Shift-Tab** → `Command::CycleFocus(forward)`.
//! 3. **`:`** → `Command::OpenPalette`.
//! 4. **Plugin activation keys** → `Command::ActivatePlugin(tag)`.
//! 5. **`KeyMap::resolve`** → whatever `Command` the binding produces.
//!
//! Focus writes and state dispatch never happen here — they're all in
//! `command`. This layer only *reads* focus (indirectly, via
//! `deliver_key_to_focused`) and produces `Command` values.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::command::{Command, KeyDelivery};
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::ui::UiState;

pub struct KeyboardHandler {
    keymap: KeyMap,
    /// First-`g`-of-`gg` flag. Lives on the handler (not the keymap)
    /// because multi-key sequences are an input-layer concern — the
    /// keymap itself is a pure lookup table.
    pending_g: bool,
}

impl KeyboardHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            keymap,
            pending_g: false,
        }
    }

    /// Expose the keymap so `app.rs` can thread it into the
    /// `DispatchCtx` it builds for `command::dispatch`.
    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// Advance the `gg` sequence state machine and resolve via the
    /// keymap. Returns the `Command` to dispatch, or `None` for a
    /// no-op (mid-sequence or unbound key).
    fn resolve_with_sequence(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Command> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(Command::Map(Action::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }

    /// Translate a raw key event into an optional `Command`. Side
    /// effects are limited to focused-surface delivery (palette filter
    /// edit, plugin state update) and focus auto-release — both
    /// performed inside `command::deliver_key_to_focused`. The caller
    /// runs `command::dispatch` on the returned `Command`.
    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ui: &mut UiState,
        center: LonLat,
    ) -> Option<Command> {
        // Resolve sequence / keymap first so the mutable borrow of
        // self.keymap (for sequence state) ends before the controller
        // reads it.
        let fallback_cmd = self.resolve_with_sequence(code, modifiers);

        // [1] Focus-first delivery — UiState owns the transition.
        match ui.deliver_key(code, modifiers, center) {
            KeyDelivery::Consumed => return None,
            KeyDelivery::Run(cmd) => return Some(cmd),
            KeyDelivery::Passthrough => {}
        }

        // [2] Focus cycling — Tab / Shift-Tab → Command::CycleFocus.
        let forward_cycle = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward_cycle = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward_cycle || backward_cycle {
            return Some(Command::CycleFocus(forward_cycle));
        }

        // [3] `:` opens the command palette (builtin, fixed key).
        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            return Some(Command::OpenPalette);
        }

        // [4] Plugin activation keys.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            return Some(Command::ActivatePlugin(tag.to_string()));
        }

        // [5] Keymap fallback.
        fallback_cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn map(action: Action) -> Command {
        Command::Map(action)
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut kb = KeyboardHandler::new(KeyMap::default());
        assert_eq!(kb.resolve_with_sequence(KeyCode::Char('g'), NONE), None);
        assert_eq!(
            kb.resolve_with_sequence(KeyCode::Char('g'), NONE),
            Some(map(Action::ZoomToWorld))
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut kb = KeyboardHandler::new(KeyMap::default());
        kb.resolve_with_sequence(KeyCode::Char('g'), NONE);
        kb.resolve_with_sequence(KeyCode::Char('h'), NONE); // breaks
        assert_eq!(kb.resolve_with_sequence(KeyCode::Char('g'), NONE), None);
    }
}
