//! Help widget — displays keybinding help as a center overlay.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Clear, Paragraph};

use crate::core::Action;
use crate::keymap::KeyMap;
use crate::ui::theme::Theme;

use super::{Plugin, PluginAction, PluginCtx};

pub struct HelpPlugin {
    active: bool,
    text: String,
}

impl Default for HelpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpPlugin {
    pub fn new() -> Self {
        Self {
            active: false,
            text: String::new(),
        }
    }

    /// Build the help text. `other_plugins` is inspected for each
    /// plugin's activation keys + description, so the listing stays in
    /// sync with the plugins actually loaded rather than a hardcoded
    /// table in this file. Help includes its own entry automatically.
    pub fn build(&mut self, keymap: &KeyMap, other_plugins: &[&dyn Plugin]) {
        let entries: Vec<(String, String)> = plugin_entries(self)
            .into_iter()
            .chain(other_plugins.iter().flat_map(|p| plugin_entries(*p)))
            .collect();

        let mut action_keys: HashMap<&str, Vec<String>> = HashMap::new();

        for (binding, action) in &keymap.bindings {
            let label = action_label(action);
            if label.is_empty() {
                continue;
            }
            let key = format_binding(binding);
            action_keys.entry(label).or_default().push(key);
        }

        let display_order: Vec<(&str, &str)> = vec![
            ("Pan", "Pan left/right/up/down"),
            ("Pan fast (horizontal)", "Fast pan left/right"),
            ("Pan fast (vertical)", "Fast pan up/down"),
            ("Zoom in", "Zoom in"),
            ("Zoom out", "Zoom out"),
            ("Zoom to world", "Zoom to world"),
            ("Reset position", "Reset position"),
            ("Quit", "Quit"),
        ];

        let mut lines = Vec::new();
        lines.push(" A terminal-based map viewer — Mapbox vector tiles".to_string());
        lines.push(" rendered as Unicode Braille.".to_string());
        lines.push(String::new());
        for (action_name, description) in &display_order {
            if let Some(keys) = action_keys.get(action_name) {
                let keys_str = keys.join(", ");
                lines.push(format!(" {:<20} {}", keys_str, description));
            }
        }

        lines.push(String::new());
        lines.push(format!(" {:<20} {}", "gg", "Zoom to world"));
        lines.push(format!(" {:<20} {}", "Tab/S-Tab", "Cycle focus"));
        for (key, description) in &entries {
            lines.push(format!(" {:<20} {}", key, description));
        }
        lines.push(String::new());
        lines.push(format!(" {:<20} {}", "Drag / Scroll", "Pan / zoom (mouse)"));
        lines.push(String::new());
        lines.push(" Bug reports and pull requests welcome:".to_string());
        lines.push(" https://github.com/Kohei-Wada/ttymap".to_string());

        self.text = lines.join("\n");
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect, theme: &Theme) {
        if map_inner.width < 20 || map_inner.height < 10 {
            return;
        }

        // Fit content with breathing room, but cap at ~80% of the map
        // area so the popup doesn't dominate the viewport.
        let lines: Vec<&str> = self.text.lines().collect();
        let content_width = lines.iter().map(|l| l.len() as u16).max().unwrap_or(30) + 6;
        let content_height = lines.len() as u16 + 2;

        let max_width = map_inner.width.saturating_sub(4).max(20);
        let max_height = map_inner.height.saturating_sub(2).max(10);
        let popup_width = content_width.clamp(50, max_width);
        let popup_height = content_height.min(max_height);

        let x = map_inner.x + (map_inner.width - popup_width) / 2;
        let y = map_inner.y + (map_inner.height - popup_height) / 2;

        let area = Rect::new(x, y, popup_width, popup_height);
        f.render_widget(Clear, area);

        let block = theme.panel("help").title_alignment(Alignment::Center);
        let widget = Paragraph::new(self.text.as_str())
            .style(theme.text())
            .block(block);
        f.render_widget(widget, area);
    }
}

impl Plugin for HelpPlugin {
    fn tag(&self) -> &str {
        "help"
    }

    fn description(&self) -> &str {
        "Toggle help"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["?"]
    }

    fn activate(&mut self, ctx: &mut PluginCtx<'_>) {
        if self.active {
            self.active = false;
            ctx.focus.release();
        } else {
            self.active = true;
            ctx.focus.take("help");
        }
    }

    fn deactivate(&mut self) {
        // Modal: losing focus means closing.
        self.active = false;
    }

    fn visible(&self) -> bool {
        self.active
    }

    fn handle_key(
        &mut self,
        _code: KeyCode,
        _modifiers: KeyModifiers,
        ctx: &mut PluginCtx<'_>,
    ) -> PluginAction {
        // Help is modal and consumes any key, releasing focus.
        self.active = false;
        ctx.focus.release();
        PluginAction::Consumed
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        HelpPlugin::render(self, f, area, theme);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("any key", "close")]
    }
}

/// `(activation_key, description)` pairs from one plugin. Empty
/// description means the plugin opted out of help listing.
fn plugin_entries(p: &dyn Plugin) -> Vec<(String, String)> {
    let desc = p.description();
    if desc.is_empty() {
        return Vec::new();
    }
    p.activation_keys()
        .into_iter()
        .map(|k| (k.to_string(), desc.to_string()))
        .collect()
}

fn action_label(action: &Action) -> &'static str {
    match action {
        Action::PanLeft | Action::PanRight | Action::PanUp | Action::PanDown => "Pan",
        Action::PanLeftFast | Action::PanRightFast => "Pan fast (horizontal)",
        Action::PanUpHalf | Action::PanDownHalf => "Pan fast (vertical)",
        Action::ZoomIn => "Zoom in",
        Action::ZoomOut => "Zoom out",
        Action::ZoomToWorld => "Zoom to world",
        Action::ResetPosition => "Reset position",
        Action::Quit => "Quit",
        _ => "",
    }
}

fn format_binding(binding: &crate::keymap::KeyBinding) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};

    let key = match binding.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Backspace => "BS".to_string(),
        _ => "?".to_string(),
    };

    if binding.modifiers.contains(KeyModifiers::CONTROL) {
        format!("C-{}", key)
    } else if binding.modifiers.contains(KeyModifiers::SHIFT) {
        format!("S-{}", key)
    } else {
        key
    }
}
