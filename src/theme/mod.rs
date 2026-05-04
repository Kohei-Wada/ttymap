//! Theme — colour data + ratatui adapter + semantic tags.
//!
//! [`ThemeId`] is the single source of truth for "which theme is
//! active": pick one from the config, derive everything else from
//! it — the [`ColorPalette`] the styler and overlays read, the
//! [`UiTheme`] the UI renders through, and the display name shown
//! to the user.
//!
//! Layout:
//! - [`palette`] — palette data (`ColorPalette` struct, `DARK` /
//!   `BRIGHT` consts). No ratatui dependency; the styler and the
//!   map renderer consume it directly.
//! - [`ui`] — ratatui adapter ([`UiTheme`]). Built from a
//!   [`ColorPalette`] at theme-switch time; consumed by the draw
//!   path.
//! - [`style`] — [`StyleKind`] semantic tags + resolver. Plugins
//!   ask for a tag string ("accent" / "muted" / …) and the bridge
//!   maps it through the active [`UiTheme`].

pub mod palette;
pub mod style;
pub mod ui;

pub use palette::{BRIGHT, ColorPalette, DARK};
pub use style::StyleKind;
pub use ui::UiTheme;

/// Identifies which theme the app is running with. Derives the concrete
/// [`ColorPalette`] and, separately, the set of styling rules consumed by
/// `styler::Styler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeId {
    #[default]
    Dark,
    Bright,
}

impl ThemeId {
    /// Parse a config string. Unknown names fall back to [`ThemeId::Dark`].
    pub fn from_name(name: &str) -> Self {
        match name {
            "bright" => Self::Bright,
            _ => Self::Dark,
        }
    }

    /// The palette this theme ships with.
    pub fn palette(self) -> &'static ColorPalette {
        match self {
            Self::Dark => &palette::DARK,
            Self::Bright => &palette::BRIGHT,
        }
    }

    /// Canonical lowercase name used for logging / `styler.name()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Bright => "bright",
        }
    }

    /// Every known theme, in the order they should appear in UI
    /// listings (command palette, help overlay). Extend here when
    /// adding a new preset; the rest of the app discovers them through
    /// this single table.
    pub fn all() -> &'static [ThemeId] {
        &[ThemeId::Dark, ThemeId::Bright]
    }
}
