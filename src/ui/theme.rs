//! UI theme — converts palette u8 values to ratatui styles.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

use crate::palette::Palette;

/// Computed UI theme from a Palette.
pub struct Theme {
    pub accent: Color,
    pub accent_alt: Color,
    pub fg: Color,
    pub muted_color: Color,
    pub bg: Color,
}

impl Theme {
    pub fn from_palette(p: &Palette) -> Self {
        Self {
            accent: Color::Indexed(p.accent),
            accent_alt: Color::Indexed(p.accent_alt),
            fg: Color::Indexed(p.fg),
            muted_color: Color::Indexed(p.muted),
            bg: Color::Indexed(p.background),
        }
    }

    pub fn panel<'a>(&self, title: &'a str) -> Block<'a> {
        Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.accent).bg(self.bg))
            .title(format!(" {} ", title))
            .style(Style::default().bg(self.bg))
    }

    pub fn text(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    pub fn muted(&self) -> Style {
        Style::default().fg(self.muted_color).bg(self.bg)
    }

    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn selected(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
}
