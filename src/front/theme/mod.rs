//! Front-side theme adapter.
//!
//! Owns [`UiTheme`] — the ratatui-aware projection of the
//! foundational [`crate::theme::ColorPalette`] data. Lives under
//! `front/` (not `theme/`) because UiTheme directly depends on
//! ratatui types; the data parts (`ColorPalette`, `StyleKind`,
//! `ThemeId`) stay at the foundation under `crate::theme` where
//! both core (renderer's styler) and front (UI chrome) consume
//! them downward.

pub mod style;
pub mod ui;

pub use style::StyleKind;
pub use ui::UiTheme;
