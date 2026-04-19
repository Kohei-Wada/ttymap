//! Keyboard input handler. Follows the industry-standard input
//! pipeline: **focus-first routing**, then global fallback chain.
//!
//! 1. [`dispatch_focused`] — the focused surface (palette or plugin)
//!    gets the event first. Returns `Some(effect)` if consumed;
//!    `None` means "focus had no interest" → fall through.
//! 2. Tab / Shift-Tab cycles focus across visible plugins.
//! 3. `:` opens the command palette.
//! 4. Plugin activation keys (`/`, `?`, `i`, …).
//! 5. `KeyMap::resolve` → `map::Action` dispatch.

use crossterm::event::{KeyCode, KeyModifiers};
use log::info;

use crate::app::InputEffect;
use crate::focus::Focus;
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::MapState;
use crate::map::render::thread::RenderHandle;
use crate::plugin::{PluginAction, PluginCtx};
use crate::ui::UiState;
use crate::ui::palette::PaletteOutcome;

/// Drive a plugin through an activation key press or palette
/// `Activate(tag)` directive. Host owns focus transitions: plugins
/// don't touch `FocusManager` on activate anymore — the host auto-takes
/// for plugins whose `wants_focus()` returns true, and handles
/// toggle-off when a visible plugin's activation key is pressed a
/// second time.
fn activate_plugin(tag: &str, ui: &mut UiState, center: LonLat) {
    // Toggle-off: re-activating the currently-focused plugin closes it.
    if ui.focus.is_plugin(tag) {
        if let Some(w) = ui.widgets.get_mut(tag) {
            w.close();
        }
        ui.focus.release();
        return;
    }

    // Normal activation.
    ui.focus.deactivate_focused(&mut ui.widgets, Some(tag));
    ui.widgets.bring_to_front(tag);

    let wants_focus = ui.widgets.get(tag).is_some_and(|w| w.wants_focus());

    let mut ctx = PluginCtx { center };
    if let Some(w) = ui.widgets.get_mut(tag) {
        w.activate(&mut ctx);
    }

    if wants_focus {
        ui.focus.take(tag.to_string());
    }
}

/// Focus-first routing. Returns `Some(effect)` when the focused
/// surface consumed the event, `None` when the host should fall
/// through to the global fallback chain (cycling, activation,
/// keymap).
fn dispatch_focused(
    code: KeyCode,
    modifiers: KeyModifiers,
    map: &mut MapState,
    ui: &mut UiState,
    render_handle: &RenderHandle,
) -> Option<InputEffect> {
    match ui.focus.current().clone() {
        Focus::Map => None,
        Focus::Palette => Some(dispatch_palette(code, modifiers, map, ui, render_handle)),
        Focus::Plugin(tag) => dispatch_plugin(&tag, code, modifiers, map, ui),
    }
}

/// Palette is modal when focused: every key is consumed. Returns the
/// `InputEffect` unconditionally.
fn dispatch_palette(
    code: KeyCode,
    modifiers: KeyModifiers,
    map: &mut MapState,
    ui: &mut UiState,
    render_handle: &RenderHandle,
) -> InputEffect {
    let center = map.center();
    let outcome = ui.palette.handle_key(code, modifiers);

    // Auto-release: palette doesn't touch focus itself; if `is_visible()`
    // dropped during handle_key (e.g. Esc / Enter closed it), the host
    // drops focus back. Mirrors the plugin auto-release rule.
    if !ui.palette.is_visible() && matches!(ui.focus.current(), Focus::Palette) {
        ui.focus.release();
    }

    match outcome {
        PaletteOutcome::Consumed | PaletteOutcome::None => InputEffect::Plugin,
        PaletteOutcome::Run(action) => {
            info!("palette: running action {:?}", action);
            if map.process_action(&action) {
                InputEffect::Map
            } else {
                InputEffect::Plugin
            }
        }
        PaletteOutcome::Activate(target_tag) => {
            info!("palette: activating plugin {:?}", target_tag);
            activate_plugin(&target_tag, ui, center);
            InputEffect::Plugin
        }
        PaletteOutcome::SetTheme(new_id) => {
            info!("palette: switching theme to {}", new_id.name());
            ui.theme_id = new_id;
            crate::theme::apply(new_id, &mut ui.theme, render_handle);
            InputEffect::Map
        }
    }
}

/// Focused plugin gets the key. May consume, jump, or pass through.
/// Host auto-releases focus when the plugin's `visible()` drops to
/// false during handle_key.
fn dispatch_plugin(
    tag: &str,
    code: KeyCode,
    modifiers: KeyModifiers,
    map: &mut MapState,
    ui: &mut UiState,
) -> Option<InputEffect> {
    let center = map.center();
    let mut ctx = PluginCtx { center };
    let outcome = match ui.widgets.get_mut(tag) {
        Some(w) => w.handle_key(code, modifiers, &mut ctx),
        None => PluginAction::Pass,
    };

    // Auto-release: if the panel closed during handle_key, drop focus.
    let still_visible = ui.widgets.get(tag).is_some_and(|w| w.visible());
    if !still_visible && ui.focus.is_plugin(tag) {
        ui.focus.release();
    }

    match outcome {
        PluginAction::Pass => None,
        PluginAction::Consumed => Some(InputEffect::Plugin),
        PluginAction::Jump(location) => {
            info!("widget: jumping to ({}, {})", location.lat, location.lon);
            map.jump_to(location);
            Some(InputEffect::Map)
        }
    }
}

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
        map: &mut MapState,
        ui: &mut UiState,
        render_handle: &RenderHandle,
    ) -> InputEffect {
        // [1] Focus-first routing.
        if let Some(effect) = dispatch_focused(code, modifiers, map, ui, render_handle) {
            return effect;
        }

        // [2] Focus cycling — Tab / Shift-Tab move across visible plugins.
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

        // [3] `:` opens the command palette (builtin, fixed key).
        if code == KeyCode::Char(':') && modifiers == KeyModifiers::NONE {
            info!("palette: opening");
            ui.focus.deactivate_focused(&mut ui.widgets, None);
            let theme_id = ui.theme_id;
            ui.palette.activate(&ui.widgets, &self.keymap, theme_id);
            ui.focus.take_palette();
            return InputEffect::Plugin;
        }

        // [4] Plugin activation keys.
        if let Some(tag) = ui.widgets.activation_tag(code, modifiers) {
            let new_tag = tag.to_string();
            activate_plugin(&new_tag, ui, map.center());
            return InputEffect::Plugin;
        }

        // [5] Keymap resolve → map.
        let action = self.keymap.resolve(code, modifiers);
        if map.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }
}
