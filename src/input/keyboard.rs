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

use crate::command::{self, Command, InputEffect, KeyDelivery};
use crate::keymap::KeyMap;
use crate::map::MapState;
use crate::map::render::thread::RenderHandle;
use crate::ui::UiState;

pub struct KeyboardHandler {
    keymap: KeyMap,
}

impl KeyboardHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self { keymap }
    }

    /// Expose the keymap so the async dispatch path in `app.rs` (and
    /// anything else that invokes `command::dispatch` outside the
    /// keyboard handler) can thread it through.
    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        map: &mut MapState,
        ui: &mut UiState,
        render_handle: &RenderHandle,
    ) -> InputEffect {
        // [1] Focus-first delivery via the controller.
        match command::deliver_key_to_focused(ui, code, modifiers, map.center()) {
            KeyDelivery::Consumed => return InputEffect::Plugin,
            KeyDelivery::Run(cmd) => {
                info!("focused: running {:?}", cmd);
                return command::dispatch(cmd, map, ui, render_handle, &self.keymap);
            }
            KeyDelivery::Passthrough => {}
        }

        // [2] Focus cycling — Tab / Shift-Tab → Command::CycleFocus.
        let forward_cycle = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward_cycle = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward_cycle || backward_cycle {
            return command::dispatch(
                Command::CycleFocus(forward_cycle),
                map,
                ui,
                render_handle,
                &self.keymap,
            );
        }

        // [3] `:` opens the command palette (builtin, fixed key).
        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            info!("palette: opening");
            return command::dispatch(Command::OpenPalette, map, ui, render_handle, &self.keymap);
        }

        // [4] Plugin activation keys.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            return command::dispatch(
                Command::ActivatePlugin(new_tag),
                map,
                ui,
                render_handle,
                &self.keymap,
            );
        }

        // [5] Keymap resolve → command.
        match self.keymap.resolve(code, modifiers) {
            Some(cmd) => command::dispatch(cmd, map, ui, render_handle, &self.keymap),
            None => InputEffect::None,
        }
    }
}
