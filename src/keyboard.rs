//! Keyboard input handler. Dispatches raw key events to widgets, then
//! asks the keymap to resolve an `Action` and routes it onwards.
//!
//! All key→`Action` translation (including the `gg` sequence) lives in
//! `KeyMap::resolve` — this handler only does dispatch.
//!
//! Key and mouse paths stay intentionally separate — they have
//! different semantics (keys are modal/captured, mouse is observer +
//! target), matching the pattern used by helix and other Rust TUI
//! apps (gitui documented a regret for unifying them).

use crossterm::event::{KeyCode, KeyModifiers};
use log::info;

use crate::app::InputEffect;
use crate::core::Core;
use crate::keymap::KeyMap;
use crate::ui::UiState;
use crate::ui::widget::WidgetAction;

pub struct KeyboardHandler {
    keymap: KeyMap,
}

impl KeyboardHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self { keymap }
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        core: &mut Core,
        ui: &mut UiState,
    ) -> InputEffect {
        let center = core.center();

        // Raw-key pass: let active widgets consume or Jump.
        for widget in ui.widgets_mut() {
            match widget.handle_key(code, modifiers, center) {
                WidgetAction::Pass => continue,
                WidgetAction::Consumed => return InputEffect::Widget,
                WidgetAction::Jump(location) => {
                    info!("widget: jumping to ({}, {})", location.lat, location.lon);
                    core.jump_to(location);
                    return InputEffect::Map;
                }
            }
        }

        // Resolve via the keymap, then let widgets claim the action
        // (SearchOpen, HelpToggle, WikiToggle), then fall through to
        // core (Pan*, Zoom*, Quit, etc.).
        let action = self.keymap.resolve(code, modifiers);
        for widget in ui.widgets_mut() {
            if widget.handle_action(&action, center) {
                return InputEffect::Widget;
            }
        }
        if core.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }
}
