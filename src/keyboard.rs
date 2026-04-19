//! Keyboard input handler. Routes raw keys to the focused widget,
//! then asks the keymap to resolve an `Action` and lets widgets claim
//! it before falling through to core.
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
use crate::ui::focus::Focus;
use crate::ui::widget::{WidgetAction, WidgetCtx};

pub struct KeyboardHandler {
    keymap: KeyMap,
}

impl KeyboardHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self { keymap }
    }

    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        core: &mut Core,
        ui: &mut UiState,
    ) -> InputEffect {
        let center = core.center();

        // Raw-key dispatch: only the focused widget sees the key.
        let focused_tag = match &ui.focus {
            Focus::Map => None,
            Focus::Widget(t) => Some(t.clone()),
        };
        if let Some(tag) = focused_tag {
            let mut ctx = WidgetCtx {
                center,
                focus: &mut ui.focus,
            };
            let outcome = match ui.widgets.get_mut(tag.as_ref()) {
                Some(w) => w.handle_key(code, modifiers, &mut ctx),
                None => WidgetAction::Pass,
            };
            match outcome {
                WidgetAction::Pass => {}
                WidgetAction::Consumed => return InputEffect::Widget,
                WidgetAction::Jump(location) => {
                    info!("widget: jumping to ({}, {})", location.lat, location.lon);
                    core.jump_to(location);
                    return InputEffect::Map;
                }
            }
        }

        // Keymap resolve → widgets claim action → core fallback.
        let action = self.keymap.resolve(code, modifiers);
        let mut ctx = WidgetCtx {
            center,
            focus: &mut ui.focus,
        };
        let mut claimed = false;
        for widget in ui.widgets.iter_mut() {
            if widget.handle_action(&action, &mut ctx) {
                claimed = true;
                break;
            }
        }
        if claimed {
            return InputEffect::Widget;
        }
        if core.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }
}
