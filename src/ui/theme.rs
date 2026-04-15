//! UI color constants and widget helpers — matched to the map's dark style.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

pub const ACCENT: Color = Color::Yellow;
pub const ACCENT_ALT: Color = Color::Cyan;
pub const FG: Color = Color::White;
pub const MUTED: Color = Color::DarkGray;
pub const BG: Color = Color::Indexed(16); // #000 — same as map background

/// Standard panel block with border and title.
pub fn panel(title: &str) -> Block<'_> {
    Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT).bg(BG))
        .title(format!(" {} ", title))
        .style(Style::default().bg(BG))
}

/// Primary text style (fg on bg).
pub fn text() -> Style {
    Style::default().fg(FG).bg(BG)
}

/// Muted text style.
pub fn muted() -> Style {
    Style::default().fg(MUTED).bg(BG)
}

/// Accent text style.
pub fn accent() -> Style {
    Style::default().fg(ACCENT)
}

/// Highlighted/selected text style.
pub fn selected() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
