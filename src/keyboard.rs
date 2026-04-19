//! Keyboard input handler. Routes raw keys to the focused widget,
//! consults the widget registry for activation triggers, and finally
//! asks the keymap to resolve a map-level `Action`.
//!
//! Widgets own their activation bindings; neither `keymap.rs` nor
//! `core/` knows any widget name. Only `KeyMap::resolve` handles the
//! `gg` sequence and user-configurable map-action bindings.

use crossterm::event::{KeyCode, KeyModifiers};
use log::info;

use crate::app::InputEffect;
use crate::core::Core;
use crate::keymap::KeyMap;
use crate::plugin::{PluginAction, PluginCtx};
use crate::ui::UiState;
use crate::ui::focus::Focus;

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

        // 1. Focused widget sees the key first. It may consume, jump,
        //    or pass it back out.
        let focused_tag = match &ui.focus {
            Focus::Map => None,
            Focus::Plugin(t) => Some(t.clone()),
        };
        if let Some(tag) = focused_tag {
            let mut ctx = PluginCtx {
                center,
                focus: &mut ui.focus,
            };
            let outcome = match ui.widgets.get_mut(tag.as_ref()) {
                Some(w) => w.handle_key(code, modifiers, &mut ctx),
                None => PluginAction::Pass,
            };
            match outcome {
                PluginAction::Pass => {}
                PluginAction::Consumed => return InputEffect::Plugin,
                PluginAction::Jump(location) => {
                    info!("widget: jumping to ({}, {})", location.lat, location.lon);
                    core.jump_to(location);
                    return InputEffect::Map;
                }
            }
        }

        // 2. Activation check: if any registered widget claims this key,
        //    deactivate whichever widget held focus (if different) so
        //    it releases any state that shouldn't outlive losing focus
        //    (e.g. wiki markers), then invoke the new widget's
        //    `activate`.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            if let Focus::Plugin(prev_tag) = ui.focus.clone()
                && prev_tag.as_ref() != new_tag.as_str()
                && let Some(prev) = ui.widgets.get_mut(prev_tag.as_ref())
            {
                prev.deactivate();
            }
            let mut ctx = PluginCtx {
                center,
                focus: &mut ui.focus,
            };
            if let Some(w) = ui.widgets.get_mut(&new_tag) {
                w.activate(&mut ctx);
                return InputEffect::Plugin;
            }
        }

        // 3. Keymap resolve → core. Plugin activation never reaches
        //    here, so `Action` only carries map-level variants.
        let action = self.keymap.resolve(code, modifiers);
        if core.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }
}
