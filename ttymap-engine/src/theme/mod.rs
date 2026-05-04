//! Theme — colour data only. The engine ships the palette table and
//! the [`ThemeId`] selector; ratatui adapters and the semantic
//! [`StyleKind`] tags live binary-side under `src/theme/` because
//! they depend on ratatui.
//!
//! [`ThemeId`] is the single source of truth for "which theme is
//! active": pick one from the config and derive everything else
//! from it.
//!
//! Layout:
//! - [`palette`] — palette data (`ColorPalette` struct, `DARK` /
//!   `BRIGHT` consts). No ratatui dependency; the styler and the
//!   map renderer consume it directly.

pub mod palette;

pub use palette::{BRIGHT, ColorPalette, DARK};

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
