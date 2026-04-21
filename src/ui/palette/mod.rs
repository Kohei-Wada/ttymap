//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! # Deliberately not a `Plugin` — do not "unify"
//!
//! Plugins and the palette look similar at a distance (both react to
//! key events, both draw popups), but they're different categories:
//!
//! - **Plugin** = a component that *contributes* functionality (one
//!   feature, self-contained state, small activation key surface).
//! - **Palette** = a *coordinator* that aggregates over the plugin
//!   registry + keymap + theme state to present a unified picker.
//!   Its whole job is to read other subsystems' state.
//!
//! Folding palette into the `Plugin` trait would work mechanically —
//! with a wider `PluginCtx<'a>` carrying `&PluginRegistry`, `&KeyMap`,
//! `ThemeId` — but it would erase that semantic distinction: every
//! plugin would gain permission to enumerate the registry (the "plugins
//! don't see each other" invariant weakens from `pub(crate)` structure
//! to a naming convention), and palette-specific concepts
//! (`SwitchProvider`, Tab-cycle exclusion) would turn into stringly-
//! typed special cases keyed on `tag == "palette"`.
//!
//! The cost of keeping it a builtin (one special field on `UiState`,
//! one well-known `SURFACE_ID = "palette"`, one `AppCommand::OpenPalette`
//! arm, one special-case branch in `UiState::deliver_to_focused_surface`)
//! is localised and tagged. The cost of unification would be spread
//! across the `Plugin` trait contract. The current asymmetry is chosen.
//!
//! # Mechanics
//!
//! Concrete behaviour (items, filter, activation) lives on a
//! [`PaletteProvider`](provider::PaletteProvider). The palette swaps
//! providers when the user picks a "sub-mode" command (e.g. "Theme"
//! switches to [`ThemeProvider`](provider::ThemeProvider)). The
//! palette never touches `FocusManager`; focus transitions are
//! driven by `UiState::open_palette` emitting
//! `FocusEvent::Claimed(SURFACE_ID)` and by the delivery path
//! emitting `FocusEvent::Released(SURFACE_ID)` when `is_visible()`
//! flips to false.

pub mod panel;
pub mod provider;
mod state;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app_command::{Effect, FocusSurface, SurfaceCtx};
use crate::color_palette::ThemeId;
use crate::keymap::KeyMap;
use crate::plugin::PluginRegistry;
use crate::theme::UiTheme;

use provider::{CommandProvider, PaletteAction};
use state::{Outcome, PaletteState};

/// `SurfaceId` for the palette in the focus system. Owned here so
/// the palette is the source of truth for its own identifier; other
/// modules import from this constant rather than hardcoding "palette".
pub const SURFACE_ID: &str = "palette";

pub struct CommandPalette {
    state: PaletteState,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            state: PaletteState::new(),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.state.is_active()
    }

    /// Open the palette with the default [`CommandProvider`]. The host
    /// is responsible for taking palette focus afterwards — the palette
    /// does not touch `FocusManager` itself (mirrors the plugin rule).
    pub fn activate(&mut self, widgets: &PluginRegistry, keymap: &KeyMap, current_theme: ThemeId) {
        let provider = Box::new(CommandProvider::new(widgets, keymap, current_theme));
        self.state.open_with(provider);
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }

    // Used by panel.rs (same module tree).
    pub(in crate::ui::palette) fn state(&self) -> &PaletteState {
        &self.state
    }
}

/// The palette is modal: every key while it is focused is `Consumed`
/// (the responder chain stops here — never falls through to the
/// background). Item selection produces `Effect::Run(AppCommand)`.
impl FocusSurface for CommandPalette {
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        _ctx: SurfaceCtx,
    ) -> Effect {
        match self.state.handle_key(code, modifiers) {
            Outcome::None | Outcome::Consumed => Effect::Consumed,
            Outcome::Run(idx) => {
                let action = self
                    .state
                    .provider
                    .as_mut()
                    .map(|p| p.execute(idx))
                    .unwrap_or(PaletteAction::Close);
                match action {
                    PaletteAction::Close => Effect::Consumed,
                    PaletteAction::SwitchProvider(next) => {
                        // Provider-to-provider transition: stay active,
                        // reopen palette with the new provider. Host
                        // sees `is_visible()` still true, keeps focus.
                        self.state.open_with(next);
                        Effect::Consumed
                    }
                    PaletteAction::Run(cmd) => Effect::Run(cmd),
                }
            }
        }
    }
}
