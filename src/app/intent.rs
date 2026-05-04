//! User-intent vocabulary consumed by [`App::dispatch`](super::App::dispatch).
//!
//! `UserIntent` is the **single enum** that anything inside the app can
//! emit to request a state change — palette providers, plugins' key
//! handlers, plugins' async pending output, the mouse adapter, and
//! (one day) external control surfaces like an HTTP/JSON-RPC front.
//! Everyone speaks the same vocabulary.
//!
//! In GoF Command-pattern terms this is the **Command** role expressed
//! as a closed algebraic type: each variant is an imperative intent
//! (`Map`, `CycleFocus`, `SetTheme`) that the Receiver ([`super::App`])
//! executes in [`App::dispatch`](super::App::dispatch). Invokers
//! (keymap, palette, plugins) **return `Vec<UserIntent>`** and never
//! execute anything themselves — the dispatcher is the sole
//! side-effect boundary. Note the naming split: "command" is reserved
//! for user-facing concepts (the CLI subcommand in `crate::commands`
//! and the `:`-palette entries), while internal intent is `UserIntent`.
//!
//! Surface activation (palette open, plugin activate) intentionally
//! does *not* live here — those are focus transitions, handled
//! internally by the
//! [`Compositor`](crate::compositor::Compositor) via
//! [`Window::open`](crate::compositor::window::Window::open) /
//! [`Window::close`](crate::compositor::window::Window::close) calls
//! from a [`Component`](crate::compositor::Component). Keeping them
//! off `UserIntent` means the focus state machine isn't coupled to the
//! dispatch table.

use crate::input::KeyMap;
use crate::map::MapAction;
use crate::theme::ThemeId;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; interpreted
/// by [`App::dispatch`](super::App::dispatch) inside the event loop.
///
/// Map-level intents are nested under [`UserIntent::Map`] because
/// [`MapState`](crate::map::MapState) owns its own command vocabulary
/// ([`MapAction`]) and consumes it through a single entry
/// ([`MapState::process_action`](crate::map::MapState::process_action)).
/// Other variants sit at the top level: each is handled directly by
/// an `App::dispatch` arm and there is no intermediate sub-system to
/// delegate to.
#[derive(Debug, Clone, PartialEq)]
pub enum UserIntent {
    /// Dispatch a map-state action (pan, zoom, reset, jump, ...).
    Map(MapAction),
    /// Stop the event loop and tear down the app. Lives at the top
    /// level (not nested under [`MapAction`]) because Quit is an
    /// app-lifetime concern, not a map-data concern — `MapState`
    /// has no business knowing whether the program is alive.
    Quit,
    /// Switch the running theme. Cross-cutting: rebuilds the styler
    /// (on the render thread) and the UI colour cache.
    SetTheme(ThemeId),
    /// Mouse cursor moved to the given terminal cell. Emitted by the
    /// mouse adapter on every event so the overlay cursor readout
    /// goes through dispatch like every other user-intent state
    /// change.
    CursorMoved(u16, u16),
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Terminal resized — update the map viewport and the render
    /// thread's canvas dimensions. Arguments are the new terminal
    /// size in cells.
    Resize(u16, u16),
    /// Write the currently displayed [`MapFrame`](crate::map::render::frame::MapFrame)
    /// to an auto-named file under `$XDG_DATA_HOME/ttymap/exports/`.
    /// Emitted by the export plugin's palette entry. Filename encodes
    /// zoom + centre + timestamp so repeated exports don't collide.
    ExportFrame,
    /// Show / hide the left sidebar. Toggling re-computes the map
    /// canvas dimensions so the render pipeline allocates the right
    /// buffer size for the visible map area.
    ToggleSidebar,
}

impl UserIntent {
    /// Resolve a `[keymap]` config name (e.g. `"quit"`, `"pan_up"`)
    /// to the matching intent. Top-level intents like [`Self::Quit`]
    /// are matched directly; everything else falls through to
    /// [`MapAction::from_config_name`] wrapped as [`Self::Map`].
    pub fn from_config_name(name: &str) -> Option<UserIntent> {
        match name {
            "quit" => Some(UserIntent::Quit),
            "toggle_sidebar" => Some(UserIntent::ToggleSidebar),
            _ => MapAction::from_config_name(name).map(UserIntent::Map),
        }
    }

    /// `(config_name, label)` pairs for every intent the help plugin
    /// surfaces. Each entry's `keymap.keys_for(intent)` is queried
    /// upstream to produce the final help text.
    pub fn listed_with_labels() -> Vec<(UserIntent, &'static str)> {
        let mut out: Vec<(UserIntent, &'static str)> = MapAction::all_listed()
            .iter()
            .map(|a| (UserIntent::Map(a.clone()), a.label()))
            .collect();
        out.push((UserIntent::Quit, "Quit"));
        out.push((UserIntent::ToggleSidebar, "Toggle sidebar"));
        out
    }

    /// Help-table entries: `(keys, label)` strings for every listed
    /// intent that has at least one key bound in `keymap`. Drives the
    /// `ttymap.help:keymap_entries()` Lua surface.
    pub fn keymap_help_entries(keymap: &KeyMap) -> Vec<(String, String)> {
        Self::listed_with_labels()
            .into_iter()
            .filter_map(|(intent, label)| {
                let keys = keymap.keys_for(&intent);
                if keys.is_empty() {
                    None
                } else {
                    Some((keys.join(", "), label.to_string()))
                }
            })
            .collect()
    }
}
