//! Semantic style tags — the bridge between "what a plugin asks for"
//! (Body / Muted / Accent / …) and the concrete `ratatui::Style` the
//! host paints with under the current theme.
//!
//! Lives next to [`UiTheme`] because resolving a tag is a property of
//! the theme, not of any plugin or widget vocabulary. Plugins (Lua)
//! pass keyword strings ("body", "muted", …) which map to a
//! [`StyleKind`] in the bridge; the bridge then resolves through the
//! active [`UiTheme`].

use ratatui::style::{Modifier, Style};

use super::UiTheme;

/// Semantic style tags plugins ask for. [`Self::resolve`] maps to a
/// concrete `ratatui::Style` under the active theme. Adding a new tag
/// requires updating both enum + resolve, no plugin signature change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleKind {
    Body,
    Muted,
    Accent,
    Highlight,
    Selected,
    Link,
    MutedFg,
}

impl StyleKind {
    /// Map a semantic tag to a concrete `ratatui::Style` under the
    /// active theme.
    pub fn resolve(self, theme: &UiTheme) -> Style {
        let base = Style::default();
        match self {
            StyleKind::Body => base.fg(theme.fg).bg(theme.bg),
            StyleKind::Muted => base.fg(theme.muted_color).bg(theme.bg),
            StyleKind::Accent => base.fg(theme.accent),
            StyleKind::Highlight => base.fg(theme.accent_alt),
            StyleKind::Selected => base.fg(theme.accent).add_modifier(Modifier::BOLD),
            StyleKind::Link => base
                .fg(theme.accent_alt)
                .bg(theme.bg)
                .add_modifier(Modifier::UNDERLINED),
            StyleKind::MutedFg => base.fg(theme.muted_color),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn body_uses_theme_fg_bg() {
        let p = &crate::theme::DARK;
        let theme = UiTheme::from_palette(p);
        let s = StyleKind::Body.resolve(&theme);
        assert_eq!(s.fg, Some(Color::Indexed(p.fg)));
        assert_eq!(s.bg, Some(Color::Indexed(p.background)));
    }

    #[test]
    fn selected_is_accent_bold() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let s = StyleKind::Selected.resolve(&theme);
        assert_eq!(s.fg, Some(Color::Indexed(crate::theme::DARK.accent)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn link_is_accent_alt_underlined() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let s = StyleKind::Link.resolve(&theme);
        assert_eq!(s.fg, Some(Color::Indexed(crate::theme::DARK.accent_alt)));
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }
}
