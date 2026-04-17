//! Help widget — displays keybinding help as a center overlay.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Clear, Paragraph};

use crate::core::input::Action;
use crate::core::keymap::KeyMap;
use crate::ui::theme::Theme;

pub struct HelpWidget {
    active: bool,
    text: String,
}

impl Default for HelpWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpWidget {
    pub fn new() -> Self {
        Self {
            active: false,
            text: String::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn build(&mut self, keymap: &KeyMap) {
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
        for (action_name, description) in &display_order {
            if let Some(keys) = action_keys.get(action_name) {
                let keys_str = keys.join(", ");
                lines.push(format!(" {:<20} {}", keys_str, description));
            }
        }

        lines.push(String::new());
        lines.push(format!(" {:<20} {}", "gg", "Zoom to world"));
        lines.push(format!(" {:<20} {}", "/", "Search location"));
        lines.push(format!(" {:<20} {}", "i", "Toggle wiki"));
        lines.push(format!(" {:<20} {}", "?", "Toggle help"));
        lines.push(String::new());
        lines.push(" Mouse:".to_string());
        lines.push(format!(" {:<20} {}", "Drag", "Pan"));
        lines.push(format!(" {:<20} {}", "Scroll", "Zoom"));

        self.text = lines.join("\n");
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect, theme: &Theme) {
        if !self.active || map_inner.width < 20 || map_inner.height < 10 {
            return;
        }

        let lines: Vec<&str> = self.text.lines().collect();
        let content_height = lines.len() as u16 + 2;
        let content_width = lines.iter().map(|l| l.len() as u16).max().unwrap_or(30) + 4;

        let popup_width = content_width.min(map_inner.width - 2);
        let popup_height = content_height.min(map_inner.height - 2);

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

fn format_binding(binding: &crate::core::keymap::KeyBinding) -> String {
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
