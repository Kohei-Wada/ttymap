//! Plugin API — surfaces the Lua bridge exposes to scripted plugins.
//!
//! Two crate-internal pieces:
//!
//! - [`map_api::MapApi`] — drawing facade for `paint_on_map`.
//! - [`layout::PanelAnchor`] / [`layout::LayoutConfig`] — anchor
//!   vocabulary for `module.layout` in scripts and for centred
//!   modal popups (help).
//!
//! The Rust-plugin author prelude that lived here previously was
//! retired together with the in-tree Rust plugins; everything in
//! `src/plugin/` now lives under `src/lua/scripts/`. Concurrency
//! primitives (`PolledFeed`, `AsyncJob`, `Throttle`) and HTTP
//! clients (`NominatimClient`) went away with that move.

pub mod layout;
pub mod map_api;

pub use map_api::MapApi;
