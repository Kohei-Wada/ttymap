//! Text styling primitives — `Modifier`, `TextStyle`, `StyleKind`.
//!
//! Colors are xterm-256 indices (`u8`), matching `ColorPalette` and
//! `MapCell`. `None` means "inherit theme default" (same semantic as
//! ratatui's `Style::default()` / reset).

use ratatui::style::{Color, Modifier as RModifier, Style as RStyle};

use crate::theme::UiTheme;

/// Bitset of text decorations. Mirror of the subset of
/// `ratatui::style::Modifier` we actually use (BOLD, UNDERLINED).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifier(u16);

impl Modifier {
    pub const NONE: Self = Self(0);
    pub const BOLD: Self = Self(1 << 0);
    pub const UNDERLINED: Self = Self(1 << 1);

    pub const fn insert(self, m: Self) -> Self {
        Self(self.0 | m.0)
    }

    pub const fn contains(self, m: Self) -> bool {
        (self.0 & m.0) == m.0
    }
}

impl From<Modifier> for RModifier {
    fn from(m: Modifier) -> Self {
        let mut r = RModifier::empty();
        if m.contains(Modifier::BOLD) {
            r |= RModifier::BOLD;
        }
        if m.contains(Modifier::UNDERLINED) {
            r |= RModifier::UNDERLINED;
        }
        r
    }
}

/// Resolved text style. `fg`/`bg` are xterm-256 indices; `None`
/// means "leave as-is" (inherit). Matches `ratatui::Style`
/// semantics on round-trip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    pub modifier: Modifier,
}

impl TextStyle {
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            modifier: Modifier::NONE,
        }
    }

    pub const fn with_fg(mut self, c: u8) -> Self {
        self.fg = Some(c);
        self
    }

    pub const fn with_bg(mut self, c: u8) -> Self {
        self.bg = Some(c);
        self
    }

    pub const fn with_modifier(mut self, m: Modifier) -> Self {
        self.modifier = m;
        self
    }
}

impl From<TextStyle> for RStyle {
    fn from(s: TextStyle) -> Self {
        let mut r = RStyle::default();
        if let Some(c) = s.fg {
            r = r.fg(Color::Indexed(c));
        }
        if let Some(c) = s.bg {
            r = r.bg(Color::Indexed(c));
        }
        r.add_modifier(s.modifier.into())
    }
}

/// Semantic style tags plugins ask for. `resolve(&UiTheme)` maps to
/// a concrete `TextStyle`. Adding a new tag requires updating both
/// enum + resolve, no plugin signature change.
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
    /// Map a semantic tag to a concrete `TextStyle` under the active
    /// theme. Must produce identical `fg`/`bg`/`modifier` to the
    /// previous `RenderWindow::body_style()` etc. accessors.
    pub fn resolve(self, theme: &UiTheme) -> TextStyle {
        let bg = color_index(theme.bg);
        let fg = color_index(theme.fg);
        let accent = color_index(theme.accent);
        let accent_alt = color_index(theme.accent_alt);
        let muted = color_index(theme.muted_color);

        match self {
            StyleKind::Body => TextStyle {
                fg,
                bg,
                modifier: Modifier::NONE,
            },
            StyleKind::Muted => TextStyle {
                fg: muted,
                bg,
                modifier: Modifier::NONE,
            },
            StyleKind::Accent => TextStyle {
                fg: accent,
                bg: None,
                modifier: Modifier::NONE,
            },
            StyleKind::Highlight => TextStyle {
                fg: accent_alt,
                bg: None,
                modifier: Modifier::NONE,
            },
            StyleKind::Selected => TextStyle {
                fg: accent,
                bg: None,
                modifier: Modifier::BOLD,
            },
            StyleKind::Link => TextStyle {
                fg: accent_alt,
                bg,
                modifier: Modifier::UNDERLINED,
            },
            StyleKind::MutedFg => TextStyle {
                fg: muted,
                bg: None,
                modifier: Modifier::NONE,
            },
        }
    }
}

/// `UiTheme` stores colors as `Color::Indexed(u8)` — extract the
/// raw index. Non-indexed variants shouldn't appear in practice
/// (constructed from palette `u8`s) but fall back to `None` rather
/// than panic.
fn color_index(c: Color) -> Option<u8> {
    match c {
        Color::Indexed(n) => Some(n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_bitset() {
        let m = Modifier::BOLD.insert(Modifier::UNDERLINED);
        assert!(m.contains(Modifier::BOLD));
        assert!(m.contains(Modifier::UNDERLINED));
        assert!(!Modifier::BOLD.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn modifier_to_ratatui() {
        let m: RModifier = Modifier::BOLD.insert(Modifier::UNDERLINED).into();
        assert!(m.contains(RModifier::BOLD));
        assert!(m.contains(RModifier::UNDERLINED));
    }

    #[test]
    fn text_style_to_ratatui_roundtrip() {
        let s = TextStyle {
            fg: Some(33),
            bg: Some(0),
            modifier: Modifier::BOLD,
        };
        let r: RStyle = s.into();
        assert_eq!(r.fg, Some(Color::Indexed(33)));
        assert_eq!(r.bg, Some(Color::Indexed(0)));
        assert!(r.add_modifier.contains(RModifier::BOLD));
    }

    #[test]
    fn text_style_default_is_all_none() {
        let s = TextStyle::default();
        let r: RStyle = s.into();
        assert_eq!(r.fg, None);
        assert_eq!(r.bg, None);
    }

    #[test]
    fn style_kind_resolve_body_uses_theme_fg_bg() {
        let p = &crate::theme::DARK;
        let theme = UiTheme::from_palette(p);
        let ours = StyleKind::Body.resolve(&theme);
        assert_eq!(ours.fg, Some(p.fg));
        assert_eq!(ours.bg, Some(p.background));
        assert_eq!(ours.modifier, Modifier::NONE);
    }

    #[test]
    fn style_kind_resolve_selected_is_accent_bold() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let ours = StyleKind::Selected.resolve(&theme);
        assert_eq!(ours.fg, Some(crate::theme::DARK.accent));
        assert!(ours.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn style_kind_resolve_link_is_accent_alt_underlined() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let ours = StyleKind::Link.resolve(&theme);
        assert_eq!(ours.fg, Some(crate::theme::DARK.accent_alt));
        assert!(ours.modifier.contains(Modifier::UNDERLINED));
    }
}
