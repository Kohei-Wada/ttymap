//! Ratatui adapter — converts palette `u8` values to ratatui styles.
//!
//! Plugins never see this type. They get [`crate::theme::StyleKind`]
//! via `RenderWindow::style()`, which resolves here.

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders};

use super::ColorPalette;

/// Computed UI theme from a [`ColorPalette`]. The `Color` fields are
/// `Color::Indexed(u8)` — xterm-256 palette entries.
///
/// Severity colours (`notify_info` / `notify_warn` / `notify_error`)
/// are TUI-side chrome only — the engine palette stays focused on
/// map features. Picked per theme so the bundled `notify.lua`'s
/// `"info"` / `"warn"` / `"error"` keywords (resolved by the Lua
/// bridge) read on both light and dark backgrounds.
pub struct UiTheme {
    pub accent: Color,
    pub accent_alt: Color,
    pub fg: Color,
    pub muted_color: Color,
    pub bg: Color,
    pub notify_info: Color,
    pub notify_warn: Color,
    pub notify_error: Color,
    /// Raw palette retained so callers (e.g. the Lua bridge's colour
    /// resolver) can access palette indices that aren't promoted to
    /// `Color` fields here.
    pub palette: ColorPalette,
}

/// xterm-256 indices for one severity tier across (info, warn, error).
struct SeverityPalette {
    info: u8,
    warn: u8,
    error: u8,
}

/// Severity tier for a dark background — saturated/bright values
/// so the popup pops on black.
const DARK_SEVERITY: SeverityPalette = SeverityPalette {
    info: 226,  // bright yellow
    warn: 208,  // orange
    error: 196, // bright red
};

/// Severity tier for a light background — darker values so the
/// popup remains legible on white. Bright yellow on white is
/// nearly invisible, hence the swap.
const BRIGHT_SEVERITY: SeverityPalette = SeverityPalette {
    info: 130,  // DarkGoldenrod3
    warn: 166,  // DarkOrange3
    error: 124, // Red3
};

/// Resolve a palette to its severity tier. Today there are two
/// presets so the dispatch is a single equality check on the bg
/// index; future light themes register here alongside the rest.
fn severity_for(p: &ColorPalette) -> &'static SeverityPalette {
    match p.background {
        231 => &BRIGHT_SEVERITY,
        _ => &DARK_SEVERITY,
    }
}

impl UiTheme {
    pub fn from_palette(p: &ColorPalette) -> Self {
        let sev = severity_for(p);
        Self {
            accent: Color::Indexed(p.accent),
            accent_alt: Color::Indexed(p.accent_alt),
            fg: Color::Indexed(p.fg),
            muted_color: Color::Indexed(p.muted),
            bg: Color::Indexed(p.background),
            notify_info: Color::Indexed(sev.info),
            notify_warn: Color::Indexed(sev.warn),
            notify_error: Color::Indexed(sev.error),
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
