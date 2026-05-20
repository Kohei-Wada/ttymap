//! `UserCommand` — the GoF Command pattern's **Command** role for
//! the entire ttymap crate.
//!
//! `UserCommand` is the **single enum** that anything inside the app
//! can emit to request a state change — palette providers, plugins'
//! key handlers, plugins' async pending output, the mouse adapter,
//! and (one day) external control surfaces like an HTTP/JSON-RPC
//! front. Every invoker speaks the same vocabulary; only the App,
//! as the GoF Receiver, executes.
//!
//! ## Layout choice: top-level module
//!
//! `UserCommand` lives at the crate root rather than inside `app/`
//! even though `App::dispatch` is the only consumer. The producers
//! span every layer (`compositor::base`, `compositor::window`,
//! `input::keymap`, `input::mouse`, `lua::api::map`,
//! `lua::api::imperative`, `palette::provider::*`, …); putting the
//! type next to its single Receiver would force every producer to
//! `use crate::app::UserCommand`, which is an upward dependency at
//! the import-graph level even though the design has nothing
//! upward about it. Keeping the vocabulary type foundational means
//! every producer reaches it via `crate::UserCommand` and no
//! crossing layer has to know that App owns the dispatch loop.
//!
//! ## Pattern roles
//!
//! - **Command** — `UserCommand` (this enum)
//! - **Invoker** — keymap, mouse adapter, palette providers, Lua
//!   callbacks, render-thread completions, …
//! - **Receiver** — [`crate::app::App`] (sole executor; see
//!   [`crate::app::App::dispatch`])
//! - **Client** — `main.rs` (composition root)
//!
//! Surface activation (palette open, plugin activate) intentionally
//! does *not* live here — those are focus transitions, handled
//! internally by the [`crate::compositor::Compositor`] via
//! [`Window::open`](crate::compositor::window::Window::open) /
//! [`Window::close`](crate::compositor::window::Window::close) calls
//! from a [`Component`](crate::compositor::Component). Keeping them
//! off `UserCommand` means the focus state machine isn't coupled to
//! the dispatch table.

use ttymap_engine::map::MapAction;
use ttymap_engine::theme::ThemeId;

/// What the app can do in response to an event. Emitted by palette
/// providers, plugin handlers, and async plugin polling; interpreted
/// by [`crate::app::App::dispatch`] inside the event loop.
///
/// Map-level commands are nested under [`UserCommand::Map`] because
/// [`MapState`](ttymap_engine::map::MapState) owns its own command vocabulary
/// ([`MapAction`]) and consumes it through a single entry
/// ([`MapState::process_action`](ttymap_engine::map::MapState::process_action)).
/// Other variants sit at the top level: each is handled directly by
/// an `App::dispatch` arm and there is no intermediate sub-system to
/// delegate to.
#[derive(Debug, Clone, PartialEq)]
pub enum UserCommand {
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
    /// goes through dispatch like every other user-command state
    /// change.
    CursorMoved(u16, u16),
    /// Cycle focus across visible plugins. `true` = forward (Tab),
    /// `false` = backward (Shift-Tab).
    CycleFocus(bool),
    /// Terminal resized — update the map viewport and the render
    /// thread's canvas dimensions. Arguments are the new terminal
    /// size in cells.
    Resize(u16, u16),
    /// Show / hide the left sidebar. Toggling re-computes the map
    /// canvas dimensions so the render pipeline allocates the right
    /// buffer size for the visible map area.
    ToggleSidebar,
    /// Toggle tile-rendered text labels (place names, road names,
    /// …) on the render thread. Geometry features keep rendering.
    /// Plugin-driven — `ttymap.map:set_labels_visible(b)` is the
    /// canonical caller (e.g. geo_quiz hard mode hides hints).
    SetLabelsVisible(bool),
    /// Show / hide every feature whose MVT `source_layer` matches
    /// `layer` on the render thread. Geometry skipped at the
    /// outermost per-tile loop, so hidden layers cost nothing per
    /// feature. Plugin-driven — `ttymap.map:set_layer_visible(name, b)`
    /// is the canonical caller (e.g. a roads-only or no-buildings
    /// view mode). Unknown layer names are accepted silently so the
    /// API stays forward-compatible with schemas added later.
    SetLayerVisible { layer: String, visible: bool },
}

impl UserCommand {
    /// Resolve a `[keymap]` config name (e.g. `"quit"`, `"pan_up"`)
    /// to the matching command. Top-level commands like [`Self::Quit`]
    /// are matched directly; everything else falls through to
    /// [`MapAction::from_config_name`] wrapped as [`Self::Map`].
    pub fn from_config_name(name: &str) -> Option<UserCommand> {
        match name {
            "quit" => Some(UserCommand::Quit),
            "toggle_sidebar" => Some(UserCommand::ToggleSidebar),
            _ => MapAction::from_config_name(name).map(UserCommand::Map),
        }
    }

    /// `(config_name, label)` pairs for every command the help plugin
    /// surfaces. Each entry's `keymap.keys_for(cmd)` is queried
    /// upstream to produce the final help text.
    pub fn listed_with_labels() -> Vec<(UserCommand, &'static str)> {
        let mut out: Vec<(UserCommand, &'static str)> = MapAction::all_listed()
            .iter()
            .map(|a| (UserCommand::Map(a.clone()), a.label()))
            .collect();
        out.push((UserCommand::Quit, "Quit"));
        out.push((UserCommand::ToggleSidebar, "Toggle sidebar"));
        out
    }
}
