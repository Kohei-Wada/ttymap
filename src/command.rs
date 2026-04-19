//! App-level command vocabulary + central dispatcher.
//!
//! `Command` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async `pending_command`, and (one day) external
//! control surfaces like an HTTP/JSON-RPC front. Everyone speaks the
//! same vocabulary.
//!
//! The dispatcher ([`dispatch`]) is the one place that knows how each
//! variant actually mutates state. Adding a new command = one new
//! variant here + one new match arm in `dispatch`. All emit sites
//! (palette outcomes, plugin actions) stay oblivious.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::focus::Focus;
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::map::render::thread::RenderHandle;
use crate::map::{Action, MapState};
use crate::plugin::{PluginAction, PluginCtx};
use crate::ui::UiState;
use crate::ui::action::UiAction;
use crate::ui::palette::PaletteOutcome;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; dispatched by
/// [`dispatch`] inside the input pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Dispatch a map-state action (pan, zoom, reset, quit, ...).
    Map(Action),
    /// Jump the map to a specific location — produced by search /
    /// here-plugin / any future picker that yields a `LonLat`.
    Jump(LonLat),
    /// Mutate UI-level state (theme, future language / export / ...).
    Ui(UiAction),
    /// Activate a plugin by its registered tag — same semantics as
    /// pressing the plugin's activation key.
    ActivatePlugin(String),
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Open the command palette with its default provider. No-op if
    /// already open.
    OpenPalette,
}

/// What a key or mouse event just changed. Drives how the main loop
/// reacts: a widget-only change redraws immediately (the map frame is
/// unchanged); a map change only requests a new render — the main
/// loop will redraw when a fresh frame arrives, avoiding a
/// stale-frame draw followed by a second fresh-frame draw.
///
/// Lives on `command` (not `app`) because it's the common return type
/// of every dispatch path — keyboard handler, command dispatcher,
/// mouse handler all share it.
#[derive(Clone, Copy, PartialEq)]
pub enum InputEffect {
    None,
    Plugin,
    Map,
}

/// Outcome of delivering a raw key event to the focused surface via
/// [`deliver_key_to_focused`]. The host routes on this:
/// `Passthrough` falls through to the global fallback chain; `Consumed`
/// is absorbed by the surface; `Run` is a `Command` for the caller to
/// dispatch next.
pub enum KeyDelivery {
    /// Focus had no claim (`Focus::Map`) or the focused plugin
    /// returned `Pass`. Caller should try the global fallback chain.
    Passthrough,
    /// Focused surface consumed the key; no `Command` to run.
    Consumed,
    /// Focused surface emitted a `Command` — caller should
    /// `dispatch` it.
    Run(Command),
}

/// Apply a command to the app. This is the single funnel for every
/// state-change intent emitted by palette / plugins / async polling.
///
/// `keymap` is threaded in because `OpenPalette` builds the default
/// `CommandProvider` which snapshots the current key bindings for
/// display hints. Other variants ignore it.
pub fn dispatch(
    cmd: Command,
    map: &mut MapState,
    ui: &mut UiState,
    render_handle: &RenderHandle,
    keymap: &KeyMap,
) -> InputEffect {
    match cmd {
        Command::Map(action) => {
            if map.process_action(&action) {
                InputEffect::Map
            } else {
                InputEffect::None
            }
        }
        Command::Jump(loc) => {
            map.jump_to(loc);
            InputEffect::Map
        }
        Command::Ui(ui_action) => {
            crate::ui::action::apply(ui_action, ui, render_handle);
            InputEffect::Map
        }
        Command::ActivatePlugin(tag) => {
            activate_plugin(&tag, ui, map.center());
            InputEffect::Plugin
        }
        Command::CycleFocus(forward) => {
            if ui.focus.cycle(&mut ui.widgets, forward) {
                InputEffect::Plugin
            } else {
                InputEffect::None
            }
        }
        Command::OpenPalette => {
            ui.focus.deactivate_focused(&mut ui.widgets, None);
            let theme_id = ui.theme_id;
            ui.palette.activate(&ui.widgets, keymap, theme_id);
            ui.focus.take_palette();
            InputEffect::Plugin
        }
    }
}

/// Route a raw key event to the currently-focused surface. The host
/// owns both key delivery and focus transitions, so every focus
/// write — take on activation, auto-release on close, release on
/// palette dismissal — lives in this module.
///
/// `center` is read once by the caller (usually from `MapState`) and
/// forwarded into `PluginCtx` so plugins can read the map viewport
/// without this function touching `MapState`.
pub fn deliver_key_to_focused(
    ui: &mut UiState,
    code: KeyCode,
    modifiers: KeyModifiers,
    center: LonLat,
) -> KeyDelivery {
    match ui.focus.current().clone() {
        Focus::Map => KeyDelivery::Passthrough,
        Focus::Palette => deliver_to_palette(ui, code, modifiers),
        Focus::Plugin(tag) => deliver_to_plugin(ui, &tag, code, modifiers, center),
    }
}

/// Deliver a key to the palette. Auto-releases focus if `handle_key`
/// dropped the palette's visibility (e.g. Esc / Enter).
fn deliver_to_palette(ui: &mut UiState, code: KeyCode, modifiers: KeyModifiers) -> KeyDelivery {
    let outcome = ui.palette.handle_key(code, modifiers);
    if !ui.palette.is_visible() && matches!(ui.focus.current(), Focus::Palette) {
        ui.focus.release();
    }
    match outcome {
        PaletteOutcome::Consumed | PaletteOutcome::None => KeyDelivery::Consumed,
        PaletteOutcome::Run(cmd) => KeyDelivery::Run(cmd),
    }
}

/// Deliver a key to a focused plugin. Auto-releases focus if the
/// plugin's `visible()` dropped during `handle_key`. `Pass` means
/// "not my key" → `Passthrough` so the host falls through to the
/// global fallback chain.
fn deliver_to_plugin(
    ui: &mut UiState,
    tag: &str,
    code: KeyCode,
    modifiers: KeyModifiers,
    center: LonLat,
) -> KeyDelivery {
    let mut ctx = PluginCtx { center };
    let outcome = match ui.widgets.get_mut(tag) {
        Some(w) => w.handle_key(code, modifiers, &mut ctx),
        None => PluginAction::Pass,
    };
    let still_visible = ui.widgets.get(tag).is_some_and(|w| w.visible());
    if !still_visible && ui.focus.is_plugin(tag) {
        ui.focus.release();
    }
    match outcome {
        PluginAction::Pass => KeyDelivery::Passthrough,
        PluginAction::Consumed => KeyDelivery::Consumed,
        PluginAction::Run(cmd) => KeyDelivery::Run(cmd),
    }
}

/// Drive a plugin through an activation request (activation key,
/// palette selection, or external command). Host owns focus
/// transitions: plugins don't touch `FocusManager` on activate — the
/// host auto-takes for plugins whose `wants_focus()` returns true,
/// and handles toggle-off when a visible plugin's activation is
/// triggered a second time.
pub fn activate_plugin(tag: &str, ui: &mut UiState, center: LonLat) {
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
