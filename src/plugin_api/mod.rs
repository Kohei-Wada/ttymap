//! Plugin API — opt-in toolbox for plugin authors.
//!
//! Distinct from the **plugin trait** ([`compositor::Component`] and
//! friends) which is the *contract* the framework calls into. The
//! contents here are the *services* a plugin chooses to call out to:
//! cross-cutting helpers that several plugins want but no single
//! subsystem owns.
//!
//! Subsystem-aware surfaces also live here when the consumer is
//! exclusively the plugin author (e.g. [`MapApi`] is a facade plugins
//! call to draw on the map; the underlying machinery lives in
//! `map/` but the API itself is plugin-facing, not map-internal).
//!
//! ## Available helpers
//!
//! - [`PolledFeed`] — `Throttle + AsyncJob` rolled together for
//!   live-data plugins (aircraft / ISS / quake / wiki / search all
//!   share the same shape).
//!
//! ## Prelude
//!
//! Most plugins want a fixed set of imports — the `Component` trait,
//! `Window` / `RenderWindow`, `Registrar`, `MapApi`, `LonLat`,
//! `AppMsg`, the widget descriptors. [`prelude`] gathers them so a
//! plugin's whole prologue collapses to:
//!
//! ```ignore
//! use crate::plugin_api::prelude::*;
//! ```

pub mod async_job;
pub mod map_api;
pub mod polled_feed;
pub mod throttle;

pub use async_job::AsyncJob;
pub use map_api::MapApi;
pub use polled_feed::PolledFeed;
// `throttle::Throttle` is consumed only by `polled_feed` today;
// re-export lands when a plugin needs raw throttle access.

/// Plugin author prelude — re-exports the items every plugin reaches
/// for. Glob-imported at the top of plugin modules so the file's
/// prologue is one line. Items are picked for "almost every plugin
/// uses this"; specialised types stay behind their full path.
///
/// `#[allow(unused_imports)]`: not every plugin uses every re-export.
/// That is the entire point of a prelude — consumers glob-in, take
/// what they need, ignore the rest.
#[allow(unused_imports)]
pub mod prelude {
    pub use super::PolledFeed;

    pub use crate::app::AppMsg;
    pub use crate::compositor::window::{RenderWindow, Window};
    pub use crate::compositor::{
        Activation, Component, Context, PaletteEntry, PaletteKind, Registrar, Task,
    };
    pub use crate::geo::LonLat;
    pub use crate::plugin_api::MapApi;
    pub use crate::widget::{
        Align, Cell, Line, List, ListItem, Paragraph, Rect, Row, Size, Span, StyleKind, Table,
        TableSel, TextStyle,
    };
}
