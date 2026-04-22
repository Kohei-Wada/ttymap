//! Widget vocabulary — neutral data types plugin code builds to
//! describe what it wants drawn. The host (`compositor`) converts
//! these descriptors to ratatui widgets at render time.
//!
//! The purpose is to keep `ratatui::*` out of plugin-facing code
//! (`src/plugin/**`, `src/palette/**`). Plugins construct e.g.
//! `widget::Paragraph { lines, style, .. }` and call
//! `win.paragraph(descriptor, rect)`; the `RenderWindow` does the
//! `From<widget::Paragraph> for ratatui::widgets::Paragraph`
//! conversion internally.
//!
//! Conversion `From<widget::X> for ratatui::X` impls live in each
//! descriptor's file. They are the **only** place in the `widget`
//! module (or on the plugin side of the codebase at all) where
//! `ratatui::*` is imported.

pub mod geom;
pub mod list;
pub mod paragraph;
pub mod style;
pub mod table;
pub mod text;

pub use geom::{Rect, Size, split_rows};
pub use list::{List, ListItem};
pub use paragraph::Paragraph;
pub use style::{StyleKind, TextStyle};
pub use table::{Cell, Row, Table, TableSel};
pub use text::{Align, Line, Span};
