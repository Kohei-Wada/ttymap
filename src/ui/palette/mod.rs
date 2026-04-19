//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! A **builtin**, not a `Plugin` — the palette coordinates across
//! plugins and other app concerns, which doesn't fit the self-contained
//! widget contract `Plugin` imposes. It lives on `UiState` like
//! `InfoOverlay` does; `keyboard.rs` routes keys to it when focus is
//! `Focus::Palette`.
//!
//! Concrete behaviour (items, filter, activation) lives on a
//! [`PaletteProvider`](provider::PaletteProvider). The palette swaps
//! providers when the user picks a "sub-mode" command (e.g. "Theme"
//! switches to [`ThemeProvider`](provider::ThemeProvider)).

pub mod panel;
pub mod provider;
mod state;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::color_palette::ThemeId;
use crate::command::Command;
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
    /// User picked an item — run the associated `Command` through
    /// `crate::command::dispatch`.
    Run(Command),
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
