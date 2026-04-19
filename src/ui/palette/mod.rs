//! Command palette — `:`-triggered popup that lists every core
//! `Action` and every visible-to-the-user plugin, filterable by
//! substring, runnable with Enter.
//!
//! A **builtin**, not a `Plugin` — the palette inherently coordinates
//! across plugins (it has to know all of them to list them), and that
//! role does not fit the self-contained widget contract `Plugin`
//! imposes. It lives on `UiState` like `InfoOverlay` does; `keyboard.rs`
//! routes keys to it explicitly when focus is `Focus::Palette`.

pub mod commands;
pub mod panel;
mod state;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::core::Action;
use crate::keymap::KeyMap;
use crate::palette::ThemeId;
use crate::plugin::PluginRegistry;
use crate::ui::focus::FocusManager;
use crate::ui::theme::Theme;

use commands::{ACTIONS, Command, CommandKind};
use state::{Outcome, PaletteState};

/// What `handle_key` wants `keyboard.rs` to do after the keystroke.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteOutcome {
    /// Key did not map to anything the palette cares about. The palette
    /// is still visible; caller should treat it as consumed so focus
    /// stays where it is.
    None,
    /// Key consumed, palette redraws.
    Consumed,
    /// User picked a core `Action` — dispatch through `core.process_action`.
    Run(Action),
    /// User picked a plugin activation — activate the plugin with this tag.
    Activate(String),
    /// User picked a theme switch — `app.rs` rebuilds the `Styler`,
    /// swaps it into the render thread, and updates `UiState.theme`.
    SetTheme(ThemeId),
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

    /// Open the palette: capture the current set of runnable commands
    /// (core actions + plugin activations) and take focus.
    pub fn activate(
        &mut self,
        focus: &mut FocusManager,
        widgets: &PluginRegistry,
        keymap: &KeyMap,
    ) {
        let commands = build_commands(widgets, keymap);
        self.state.set_commands(commands);
        self.state.open();
        focus.take_palette();
    }

    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        focus: &mut FocusManager,
    ) -> PaletteOutcome {
        let outcome = self.state.handle_key(code, modifiers);
        if !self.state.is_active() {
            focus.release();
        }
        match outcome {
            Outcome::None => PaletteOutcome::None,
            Outcome::Consumed => PaletteOutcome::Consumed,
            Outcome::Run(idx) => match self.state.commands[idx].kind.clone() {
                CommandKind::Action(a) => PaletteOutcome::Run(a),
                CommandKind::Activate(tag) => PaletteOutcome::Activate(tag),
                CommandKind::SetTheme(t) => PaletteOutcome::SetTheme(t),
            },
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
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

fn build_commands(widgets: &PluginRegistry, keymap: &KeyMap) -> Vec<Command> {
    let mut commands: Vec<Command> = Vec::new();

    for (label, action) in ACTIONS {
        commands.push(Command {
            label: (*label).to_string(),
            keys: keymap.keys_for(action).join(", "),
            kind: CommandKind::Action(action.clone()),
        });
    }

    for p in widgets.iter() {
        let description = p.description();
        if description.is_empty() {
            continue;
        }
        commands.push(Command {
            label: description.to_string(),
            keys: p.activation_keys().join(", "),
            kind: CommandKind::Activate(p.tag().to_string()),
        });
    }

    for theme in ThemeId::all() {
        commands.push(Command {
            label: format!("Theme: {}", theme.name()),
            keys: String::new(),
            kind: CommandKind::SetTheme(*theme),
        });
    }

    commands
}
