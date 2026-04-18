//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

// The crate is primarily a binary (see src/main.rs). The library exists so
// tests can share a module tree; external consumers only need the handful
// of items used by main.rs, so everything else is `pub(crate)`.

/// Application event loop and terminal I/O orchestration.
pub mod app;

/// Settings loaded from `~/.config/ttymap/config.toml` + CLI overrides.
pub mod config;

/// Core state management — input, keymap, map state snapshots.
pub mod core;

/// File-based logging to XDG state directory.
pub mod logging;

/// Mapbox GL style JSON parser — filter compilation, color resolution.
pub mod styler;

// ── Internal modules (not part of the external surface) ──────────────────

pub(crate) mod geo;
pub(crate) mod geocode;
pub(crate) mod nominatim;
pub(crate) mod palette;
pub(crate) mod render;
pub(crate) mod shared;
pub(crate) mod tile;
pub(crate) mod ui;
pub(crate) mod wikipedia;
