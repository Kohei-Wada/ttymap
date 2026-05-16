//! ttymap-shared — leaf vocabulary every crate above the engine
//! consumes.
//!
//! Holds the cross-cutting vocabularies that would otherwise force a
//! circular dependency between UI (`ttymap-tui`), plugin runtime
//! (`ttymap-lua`), and binary entry (`ttymap-app`):
//!
//! - [`command::UserCommand`] — the single Command vocabulary every
//!   emission site (palette, plugin callbacks, mouse adapter, …)
//!   speaks and that `ttymap-app::App::dispatch` interprets.
//! - [`event::EventBus`] / [`event::Event`] — Lua-agnostic pub/sub
//!   primitive at the integration point between Rust core and Lua
//!   plugin runtime.
//!
//! The runtime `Config` shape (wraps `ttymap_engine::Config` with
//! binary-side knobs) plus the user-facing `KeybindingOverrides`
//! settings map live in `ttymap-config`. The live `KeyMap` that
//! resolves a keypress to a `UserCommand` lives in
//! `ttymap-tui::input::keymap` (where the crossterm dependency
//! that backs `KeyCode` belongs).
//!
//! Engine dependency is one-way: this crate uses `ttymap-engine`
//! types (`LonLat`, `ThemeId`, `MapAction`) but never the reverse.
//! ratatui / crossterm do not enter here.

pub mod command;
pub mod event;

pub use command::UserCommand;
