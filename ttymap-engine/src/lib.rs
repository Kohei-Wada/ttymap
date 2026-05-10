//! `ttymap-engine` — headless rendering engine for ttymap.
//!
//! Given a [`map::Viewport`] and a [`Styler`](map::styler::Styler),
//! the engine produces [`map::render::frame::MapFrame`]s — completed
//! grids of `MapCell { ch, fg, bg }`. The engine knows nothing about
//! ratatui, terminals, or any UI framework. The binary that owns the
//! event loop wraps these frames into a `Widget` for ratatui.
//!
//! Concretely, this crate exposes:
//!
//! - **`map/`** — viewport + map state + render pipeline (tile fetch,
//!   decode, draw, label, polygon fill, Braille pack) + styler schema
//!   + tile cache
//! - **`theme/`** — colour palette data ([`theme::ColorPalette`],
//!   [`theme::ThemeId`], `DARK` / `BRIGHT` consts). No `UiTheme` /
//!   `StyleKind` — those are ratatui adapters and live in the binary.
//! - **`geo`** — Web Mercator projection math
//! - **`shared::http`** — User-Agent-tagged reqwest wrapper, used by
//!   the tile fetcher and (via re-import) by the binary's Lua bridge
//! - **[`Config`]** — engine-side settings (cache, map initial view,
//!   render style/language). The binary wraps this with its own
//!   runtime fields.

pub mod config;
pub mod error;
pub mod geo;
pub mod map;
pub mod shared;
pub mod theme;

pub use config::Config;
pub use error::EngineError;
