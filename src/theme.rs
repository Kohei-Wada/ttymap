//! UI theme — converts palette u8 values to ratatui styles.

use std::sync::Arc;

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

use crate::color_palette::{ColorPalette, ThemeId};
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;

/// Computed UI theme from a ColorPalette.
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

/// Runtime theme switch: build a fresh `Styler` for `new_id`, push it
/// into the render thread, and refresh the passed-in UI `UiTheme` in
/// place. The caller keeps its own `theme_id` source of truth in sync.
pub fn apply(new_id: ThemeId, ui_theme: &mut UiTheme, render_handle: &RenderHandle) {
    let styler = Arc::new(Styler::new(new_id));
    render_handle.set_styler(styler.clone());
    *ui_theme = UiTheme::from_palette(styler.palette());
}
