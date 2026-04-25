//! Plugin API — everything a plugin author reaches for.
//!
//! Distinct from the **plugin trait** ([`compositor::Component`] and
//! friends) which is the *contract* the framework calls into. The
//! contents here are the *services* a plugin chooses to call out to.
//!
//! Two flavours of inhabitant, both legitimate:
//!
//! - **framework helpers** — generic primitives several plugins
//!   share (`PolledFeed`, `AsyncJob`, `Throttle`) plus subsystem
//!   facades (`MapApi`).
//! - **shared service clients** — wrappers around external APIs
//!   that *only plugins* call (`NominatimClient`). Service clients
//!   that also serve non-plugin code (HTTP transport, geoip used by
//!   the snap CLI) live in [`crate::shared`] instead.
//!
//! The split between this module and `shared/` is by **consumer
//! scope**: anything 100% plugin-only is here; anything plugin +
//! host is in `shared/`. Plugin-private clients (one plugin's HTTP
//! parser) stay inside that plugin's folder.
//!
//! ## Layout convention
//!
//! Currently flat (~5 files). When the file count exceeds ~10,
//! split into sub-folders along the framework / drawing / services
//! axes:
//!
//! ```text
//! plugin_api/
//! ├── concurrency/    async_job, throttle, polled_feed
//! ├── drawing/        map_api
//! └── services/       nominatim, ...
//! ```
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
pub mod layout;
pub mod map_api;
pub mod nominatim;
pub mod panel;
pub mod polled_feed;
pub mod throttle;

pub use async_job::AsyncJob;
pub use layout::{LayoutConfig, PanelAnchor};
pub use map_api::MapApi;
pub use panel::ListPanel;
pub use polled_feed::PolledFeed;
// `throttle::Throttle` is consumed only by `polled_feed` today;
// re-export lands when a plugin needs raw throttle access.
// `nominatim::*` consumers (search, info) import the full path
// for clarity since the types include `NominatimClient`,
// `SearchResult`, `PlaceInfo`.

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
    pub use super::{LayoutConfig, ListPanel, PanelAnchor, PolledFeed};

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
