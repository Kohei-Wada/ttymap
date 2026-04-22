//! UI theme — converts palette u8 values to ratatui styles.
//!
//! Plugins never see this type. They get [`widget::StyleKind`] via
//! `RenderWindow::style()`, which resolves here. Map of concrete
//! styles is private to the host; this file is the only place in
//! plugin-facing territory where `ratatui::style::*` is touched
//! (apart from the `widget::*` conversion floor).

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};

use crate::color_palette::ColorPalette;

/// Computed UI theme from a ColorPalette. The five `Color` fields
/// are `Color::Indexed(u8)` — xterm-256 palette entries.
pub struct UiTheme {
    pub accent: Color,
    pub accent_alt: Color,
    pub fg: Color,
    pub muted_color: Color,
    pub bg: Color,
}

impl UiTheme {
    pub fn from_palette(p: &ColorPalette) -> Self {
        Self {
            accent: Color::Indexed(p.accent),
            accent_alt: Color::Indexed(p.accent_alt),
            fg: Color::Indexed(p.fg),
            muted_color: Color::Indexed(p.muted),
            bg: Color::Indexed(p.background),
        }
    }

    /// Build a theme-styled bordered block with `title`. Used by
    /// `RenderWindow::panel` and `widget::Paragraph::into_ratatui`
    /// (for framed_title).
    pub fn panel(&self, title: &str) -> Block<'static> {
        Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.accent).bg(self.bg))
            .title(format!(" {} ", title))
            .style(Style::default().bg(self.bg))
    }
}
