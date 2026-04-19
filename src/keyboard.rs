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

        // 2. Focus cycling keys. The focused plugin gets these first
        //    via step 1 — wiki consumes C-j/C-k for article nav,
        //    search swallows Tab in its query, etc. When focus is
        //    elsewhere (or the focused plugin passes), cycle focus
        //    through visible plugins.
        let ctrl = modifiers == KeyModifiers::CONTROL;
        let forward_cycle = (code == KeyCode::Tab && modifiers == KeyModifiers::NONE)
            || (ctrl && code == KeyCode::Char('j'));
        let backward_cycle = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT))
            || (ctrl && code == KeyCode::Char('k'));
        if forward_cycle {
            return cycle_focus(ui, true);
        }
        if backward_cycle {
            return cycle_focus(ui, false);
        }

        // 3. Activation check: if any registered plugin claims this
        //    key, deactivate whichever plugin held focus (if
        //    different) so it releases any state that shouldn't
        //    outlive losing focus (e.g. wiki markers), then invoke
        //    the new plugin's `activate`.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            if let Focus::Plugin(prev_tag) = ui.focus.clone()
                && prev_tag.as_ref() != new_tag.as_str()
                && let Some(prev) = ui.widgets.get_mut(prev_tag.as_ref())
            {
                prev.deactivate();
            }
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

/// Move focus to the next (or previous) visible plugin, wrapping
/// through Map. Cycle order: Map → visible[0] → … → visible[last] →
/// Map → … (reverse swaps the ends).
fn cycle_focus(ui: &mut UiState, forward: bool) -> InputEffect {
    let visible: Vec<String> = ui
        .widgets
        .iter()
        .filter(|w| w.visible())
        .map(|w| w.tag().to_string())
        .collect();

    if visible.is_empty() {
        return InputEffect::None;
    }

    let next: Option<String> = match &ui.focus {
        // From Map, enter at the appropriate end of the list.
        Focus::Map => Some(if forward {
            visible.first().unwrap().clone()
        } else {
            visible.last().unwrap().clone()
        }),
        Focus::Plugin(cur) => {
            let cur_str = cur.as_ref();
            match visible.iter().position(|t| t == cur_str) {
                Some(i) if forward => {
                    if i + 1 < visible.len() {
                        Some(visible[i + 1].clone())
                    } else {
                        None // past last → Map
                    }
                }
                Some(i) => {
                    if i > 0 {
                        Some(visible[i - 1].clone())
                    } else {
                        None // before first → Map
                    }
                }
                // Current focus not visible — enter the list at the edge.
                None => Some(if forward {
                    visible.first().unwrap().clone()
                } else {
                    visible.last().unwrap().clone()
                }),
            }
        }
    };

    // Deactivate the currently-focused plugin (no-op for non-modal).
    if let Focus::Plugin(prev) = ui.focus.clone()
        && let Some(p) = ui.widgets.get_mut(prev.as_ref())
    {
        p.deactivate();
    }

    ui.focus = match next {
        Some(tag) => Focus::Plugin(tag.into()),
        None => Focus::Map,
    };
    InputEffect::Plugin
}
