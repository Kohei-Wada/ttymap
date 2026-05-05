//! Ratatui adapter — converts palette `u8` values to ratatui styles.
//!
//! Plugins never see this type. They get [`crate::theme::StyleKind`]
//! via `RenderWindow::style()`, which resolves here.

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};

use super::ColorPalette;

/// Computed UI theme from a [`ColorPalette`]. The five `Color` fields
/// are `Color::Indexed(u8)` — xterm-256 palette entries.
pub struct UiTheme {
    pub accent: Color,
    pub accent_alt: Color,
    pub fg: Color,
    pub muted_color: Color,
    pub bg: Color,
    /// Raw palette retained so callers (e.g. the Lua bridge's colour
    /// resolver) can access palette indices that aren't promoted to
    /// `Color` fields here.
    pub palette: ColorPalette,
}

impl UiTheme {
    pub fn from_palette(p: &ColorPalette) -> Self {
        Self {
            accent: Color::Indexed(p.accent),
            accent_alt: Color::Indexed(p.accent_alt),
            fg: Color::Indexed(p.fg),
            muted_color: Color::Indexed(p.muted),
            bg: Color::Indexed(p.background),
            palette: ColorPalette { ..*p },
        }
    }

    /// Build a theme-styled bordered block with `title`. Used by
    /// `RenderWindow::panel` to wrap content in a framed container.
    /// Unfocused panels get a subtle muted border so a stack of
    /// three sidebar cards doesn't look like a wall of yellow;
    /// focused panels switch to `accent` so the active section
    /// pops out.
    pub fn panel(&self, title: &str, focused: bool) -> Block<'static> {
        let border = if focused {
            self.accent
        } else {
            self.muted_color
        };
        Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border).bg(self.bg))
            .title(format!(" {} ", title))
            .style(Style::default().bg(self.bg))
    }
}
