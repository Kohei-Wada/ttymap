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
use crate::ui::palette::PaletteOutcome;

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

        // 1a. Palette (builtin) has first dibs if focused.
        if ui.focus.is_palette() {
            let outcome = ui.palette.handle_key(code, modifiers, &mut ui.focus);
            match outcome {
                PaletteOutcome::Consumed | PaletteOutcome::None => {
                    return InputEffect::Plugin;
                }
                PaletteOutcome::Run(action) => {
                    info!("palette: running action {:?}", action);
                    return if core.process_action(&action) {
                        InputEffect::Map
                    } else {
                        InputEffect::Plugin
                    };
                }
                PaletteOutcome::Activate(target_tag) => {
                    info!("palette: activating plugin {:?}", target_tag);
                    ui.focus
                        .deactivate_focused(&mut ui.widgets, Some(&target_tag));
                    ui.widgets.bring_to_front(&target_tag);
                    let mut ctx = PluginCtx {
                        center,
                        focus: &mut ui.focus,
                    };
                    if let Some(w) = ui.widgets.get_mut(&target_tag) {
                        w.activate(&mut ctx);
                    }
                    return InputEffect::Plugin;
                }
            }
        }

        // 1b. Focused plugin sees the key. It may consume, jump, or
        //     pass it back out.
        let focused_tag = match ui.focus.current() {
            Focus::Map | Focus::Palette => None,
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

        // 2. Focus cycling keys. The focused plugin gets these first
        //    via step 1 — search swallows Tab in its query, etc. When
        //    focus is elsewhere (or the focused plugin passes), cycle
        //    focus through visible plugins.
        let forward_cycle = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward_cycle = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward_cycle || backward_cycle {
            return if ui.focus.cycle(&mut ui.widgets, forward_cycle) {
                InputEffect::Plugin
            } else {
                InputEffect::None
            };
        }

        // 3. Palette is a builtin with a fixed, non-overridable key.
        //    Opens from any focus (Map or another plugin); the latter
        //    closes first via `deactivate_focused`.
        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            info!("palette: opening");
            ui.focus.deactivate_focused(&mut ui.widgets, None);
            ui.palette
                .activate(&mut ui.focus, &ui.widgets, &self.keymap);
            return InputEffect::Plugin;
        }

        // 4. Activation check: if any registered plugin claims this
        //    key, release whichever plugin held focus (unless it's the
        //    same one, so toggling re-activation works) and invoke
        //    the new plugin's `activate`.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            ui.focus.deactivate_focused(&mut ui.widgets, Some(&new_tag));
            // Newest activation renders on top (z-order).
            ui.widgets.bring_to_front(&new_tag);
            let mut ctx = PluginCtx {
                center,
                focus: &mut ui.focus,
            };
            if let Some(w) = ui.widgets.get_mut(&new_tag) {
                w.activate(&mut ctx);
                return InputEffect::Plugin;
            }
        }

        // 4. Keymap resolve → core. Plugin activation never reaches
        //    here, so `Action` only carries map-level variants.
        let action = self.keymap.resolve(code, modifiers);
        if core.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }
}
