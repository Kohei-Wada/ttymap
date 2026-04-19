//! Keyboard input handler. Pure **input router**: translates raw key
//! events into `Command`s and hands them to `command::dispatch`.
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
//! Focus writes never happen here — they're all in `command`. This
//! layer only *reads* focus (indirectly, via `deliver_key_to_focused`).

use crossterm::event::{KeyCode, KeyModifiers};
use log::info;

use crate::command::{self, Command, DispatchCtx, InputEffect, KeyDelivery};
use crate::keymap::KeyMap;
use crate::map::render::thread::RenderHandle;
use crate::map::{Action, MapState};
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

    /// Expose the keymap so the async dispatch path in `app.rs` (and
    /// anything else that invokes `command::dispatch` outside the
    /// keyboard handler) can thread it through.
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

    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        map: &mut MapState,
        ui: &mut UiState,
        render_handle: &RenderHandle,
    ) -> InputEffect {
        // Resolve sequence / keymap **before** building the DispatchCtx
        // so the `&mut self.keymap` path (sequence state flip) can
        // finish before ctx reborrows `&self.keymap`.
        let fallback_cmd = self.resolve_with_sequence(code, modifiers);

        let mut ctx = DispatchCtx {
            map,
            ui,
            render_handle,
            keymap: &self.keymap,
        };

        // [1] Focus-first delivery via the controller.
        match command::deliver_key_to_focused(&mut ctx, code, modifiers) {
            KeyDelivery::Consumed => return InputEffect::Plugin,
            KeyDelivery::Run(cmd) => {
                info!("focused: running {:?}", cmd);
                return command::dispatch(cmd, &mut ctx);
            }
            KeyDelivery::Passthrough => {}
        }

        // [2] Focus cycling — Tab / Shift-Tab → Command::CycleFocus.
        let forward_cycle = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward_cycle = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward_cycle || backward_cycle {
            return command::dispatch(Command::CycleFocus(forward_cycle), &mut ctx);
        }

        // [3] `:` opens the command palette (builtin, fixed key).
        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            info!("palette: opening");
            return command::dispatch(Command::OpenPalette, &mut ctx);
        }

        // [4] Plugin activation keys.
        if let Some(tag) = ctx.ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            return command::dispatch(Command::ActivatePlugin(new_tag), &mut ctx);
        }

        // [5] Keymap fallback — dispatch the pre-resolved command.
        match fallback_cmd {
            Some(cmd) => command::dispatch(cmd, &mut ctx),
            None => InputEffect::None,
        }
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
