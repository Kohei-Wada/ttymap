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
//! one `Focus::Palette` variant, two `FocusEvent::Palette*` events,
//! one `AppMsg::OpenPalette` arm) is localised and tagged. The cost
//! of unification would be spread across the `Plugin` trait contract.
//! The current asymmetry is chosen.
//!
//! # Mechanics
//!
//! Concrete behaviour (items, filter, activation) lives on a
//! [`PaletteProvider`](provider::PaletteProvider). The palette swaps
//! providers when the user picks a "sub-mode" command (e.g. "Theme"
//! switches to [`ThemeProvider`](provider::ThemeProvider)). The
//! palette never touches `FocusManager`; focus transitions are
//! driven by `UiState::open_palette` emitting `FocusEvent::PaletteOpened`
//! and by the delivery path emitting `FocusEvent::PaletteClosed` when
//! `is_visible()` flips to false.

pub mod panel;
pub mod provider;
mod state;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app_msg::AppMsg;
use crate::color_palette::ThemeId;
use crate::keymap::KeyMap;
use crate::plugin::PluginRegistry;
use crate::theme::UiTheme;

use provider::{CommandProvider, PaletteAction};
use state::{Outcome, PaletteState};

/// What `handle_key` wants `keyboard.rs` to do after the keystroke.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteOutcome {
    /// Key did not map to anything the palette cares about. Palette is
    /// still visible; caller treats it as consumed so focus stays.
    None,
    /// Key consumed, palette redraws.
    Consumed,
    /// User picked an item — run the associated `AppMsg` through
    /// `crate::app_msg::dispatch`.
    Run(AppMsg),
}

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

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> PaletteOutcome {
        let outcome = self.state.handle_key(code, modifiers);
        match outcome {
            Outcome::None => PaletteOutcome::None,
            Outcome::Consumed => PaletteOutcome::Consumed,
            Outcome::Run(idx) => {
                let action = self
                    .state
                    .provider
                    .as_mut()
                    .map(|p| p.execute(idx))
                    .unwrap_or(PaletteAction::Close);
                match action {
                    PaletteAction::Close => PaletteOutcome::Consumed,
                    PaletteAction::SwitchProvider(next) => {
                        // Provider-to-provider transition: stay active,
                        // reopen palette with the new provider. Host
                        // sees `is_visible()` still true, keeps focus.
                        self.state.open_with(next);
                        PaletteOutcome::Consumed
                    }
                    PaletteAction::Run(cmd) => PaletteOutcome::Run(cmd),
                }
            }
        }
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
