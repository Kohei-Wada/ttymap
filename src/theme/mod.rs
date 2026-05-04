//! Theme — binary-side ratatui adapter and semantic-tag resolver.
//!
//! Colour data lives in [`ttymap_engine::theme`] (`ColorPalette`,
//! `ThemeId`, `DARK`/`BRIGHT` consts). It's re-exported here so the
//! rest of the binary can keep using `crate::theme::*` without
//! caring that the data half lives in a sibling crate.
//!
//! Layout:
//! - [`ui`] — ratatui adapter ([`UiTheme`]). Built from a
//!   [`ColorPalette`] at theme-switch time; consumed by the draw
//!   path.
//! - [`style`] — [`StyleKind`] semantic tags + resolver. Plugins
//!   ask for a tag string ("accent" / "muted" / …) and the bridge
//!   maps it through the active [`UiTheme`].

pub mod style;
pub mod ui;

pub use style::StyleKind;
pub use ui::UiTheme;

pub use ttymap_engine::theme::{BRIGHT, ColorPalette, DARK, ThemeId};
