//! Background responder — the host's "default" key handler when no
//! palette / plugin has focus.
//!
//! Owns the four global key behaviours that used to live as router
//! stages:
//! - Tab / Shift-Tab → cycle focus across visible plugins
//! - `:` → open the command palette
//! - plugin activation keys (`/`, `i`, `?`, …) → activate
//! - keymap fallback (h/j/k/l/q/0/+/-/…) → map action
//!
//! Plus the `gg` multi-key sequence state machine.
//!
//! Modelled as a separate type (rather than folding into `MapState`)
//! so the map domain stays pure — `MapState` only knows `Action`.
//! Knowledge of palette, plugin activation, and focus cycling lives
//! here, one layer above domain.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::{AppCommand, Effect};
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::ui::UiState;

pub struct BackgroundResponder {
    keymap: KeyMap,
    /// First-`g` flag of the `gg` sequence. Lives here (not in
    /// `KeyMap`) because multi-key sequencing is a responder concern;
    /// the keymap itself is a stateless lookup table.
    pending_g: bool,
}

impl BackgroundResponder {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            keymap,
            pending_g: false,
        }
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// Advance the `gg` state machine and resolve via the keymap.
    /// Returns the `AppCommand` this keypress completes (if any).
    ///
    /// **Always called from the router**, even when a focused surface
    /// will end up consuming the key — vim semantics: typing into the
    /// palette must break a Normal-mode `gg` so the second `g` does
    /// not unexpectedly fire `ZoomToWorld` after the palette closes.
    pub fn resolve_keymap(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<AppCommand> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(AppCommand::Map(Action::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }

    /// Handle the key as the background responder. Tries Tab / `:` /
    /// plugin activation / keymap fallback in priority order; returns
    /// `Effect::Pass` if nothing matched.
    ///
    /// `keymap_fallback` is the precomputed result of
    /// [`Self::resolve_keymap`] — the router computes it eagerly so
    /// the `gg` sequence advances consistently across responders.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ui: &UiState,
        keymap_fallback: Option<AppCommand>,
    ) -> Effect {
        let forward = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward || backward {
            return Effect::Run(AppCommand::CycleFocus(forward));
        }

        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            return Effect::Run(AppCommand::OpenPalette);
        }

        if let Some(tag) = ui.focus.widgets().activation_tag(code, modifiers) {
            return Effect::Run(AppCommand::ActivatePlugin(tag.to_string()));
        }

        if let Some(cmd) = keymap_fallback {
            return Effect::Run(cmd);
        }

        Effect::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn map(action: Action) -> AppCommand {
        AppCommand::Map(action)
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut bg = BackgroundResponder::new(KeyMap::default());
        assert_eq!(bg.resolve_keymap(KeyCode::Char('g'), NONE), None);
        assert_eq!(
            bg.resolve_keymap(KeyCode::Char('g'), NONE),
            Some(map(Action::ZoomToWorld))
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut bg = BackgroundResponder::new(KeyMap::default());
        bg.resolve_keymap(KeyCode::Char('g'), NONE);
        bg.resolve_keymap(KeyCode::Char('h'), NONE); // breaks
        assert_eq!(bg.resolve_keymap(KeyCode::Char('g'), NONE), None);
    }
}
