//! ttymap-core — shared types every other crate in the workspace
//! consumes.
//!
//! Holds the cross-cutting vocabularies and configuration shape that
//! would otherwise force a circular dependency between UI (`ttymap-tui`),
//! plugin runtime (`ttymap-lua`), and binary entry (`ttymap-app`):
//!
//! - [`command::UserCommand`] — the single Command vocabulary every
//!   emission site (palette, plugin callbacks, mouse adapter, …)
//!   speaks and that `ttymap-app::App::dispatch` interprets.
//! - [`event::EventBus`] / [`event::Event`] — Lua-agnostic pub/sub
//!   primitive at the integration point between Rust core and Lua
//!   plugin runtime.
//! - [`keymap::KeyMap`] / [`keymap::KeybindingOverrides`] — the
//!   `key → UserCommand` resolution table. Lives here because it
//!   names both the Command vocabulary and the user-configurable
//!   override shape.
//!
//! The runtime `Config` shape (wraps `ttymap_engine::Config` with
//! binary-side knobs) lives in `ttymap-config`, depending on this
//! crate for `KeybindingOverrides`.
//!
//! Engine dependency is one-way: this crate uses `ttymap-engine`
//! types (`LonLat`, `ThemeId`, `MapAction`) but never the reverse.
//! ratatui / crossterm beyond key codes don't enter here.

pub mod command;
pub mod event;
pub mod keymap;

pub use command::UserCommand;
