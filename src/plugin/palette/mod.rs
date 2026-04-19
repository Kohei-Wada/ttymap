//! Command palette widget — `:`-triggered popup that lets the user
//! discover and invoke any registered action (core `Action` variants +
//! plugin activations) by typing a substring of the label.
//!
//! Self-contained in the usual widget pattern: owns state, handles key
//! events, returns a `PluginAction` for the keyboard dispatcher to
//! forward. Commands are captured at `build` time from the current
//! keymap + the list of other plugins, mirroring how `HelpPlugin`
//! snapshots its help text.

pub mod commands;
pub mod panel;
mod state;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::keymap::KeyMap;
use crate::ui::theme::Theme;

use commands::{ACTIONS, Command, CommandKind};
use state::{Outcome, PaletteState};

use super::{Plugin, PluginAction, PluginCtx};

pub struct PalettePlugin {
    pub(in crate::plugin::palette) state: PaletteState,
}

impl Default for PalettePlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PalettePlugin {
    pub fn new() -> Self {
        Self {
            state: PaletteState::new(),
        }
    }

    /// Populate the command list from the current keymap and the other
    /// registered plugins. Call after all plugins are constructed (same
    /// pattern help uses). Safe to call again to refresh if bindings
    /// change.
    pub fn build(&mut self, keymap: &KeyMap, other_plugins: &[&dyn Plugin]) {
        let mut commands: Vec<Command> = Vec::new();

        for (label, action) in ACTIONS {
            commands.push(Command {
                label: (*label).to_string(),
                keys: keymap.keys_for(action).join(", "),
                kind: CommandKind::Action(action.clone()),
            });
        }

        for p in other_plugins {
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

        self.state.set_commands(commands);
    }
}

impl Plugin for PalettePlugin {
    fn tag(&self) -> &str {
        "palette"
    }

    fn description(&self) -> &str {
        "Command palette"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec![":"]
    }

    fn activate(&mut self, ctx: &mut PluginCtx<'_>) {
        self.state.open();
        ctx.focus.take("palette");
    }

    fn deactivate(&mut self) {
        self.state.close();
    }

    fn visible(&self) -> bool {
        self.state.is_active()
    }

    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut PluginCtx<'_>,
    ) -> PluginAction {
        let outcome = self.state.handle_key(code, modifiers);
        if !self.state.is_active() {
            ctx.focus.release();
        }
        match outcome {
            Outcome::None | Outcome::Consumed => PluginAction::Consumed,
            Outcome::Run(idx) => match self.state.commands[idx].kind.clone() {
                CommandKind::Action(a) => PluginAction::RunAction(a),
                CommandKind::Activate(tag) => PluginAction::Activate(tag),
            },
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        panel::render_panel(self, f, area, theme);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }
}
