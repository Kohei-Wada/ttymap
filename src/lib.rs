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

/// Keyboard event handler — raw key dispatch to widgets + Action
/// translation + fallback to core.
pub(crate) mod keyboard;

/// Key binding table and TOML override shape.
pub(crate) mod keymap;

/// Mouse event handler — translates crossterm mouse events into
/// core/UI updates. Key input lives elsewhere; keeping the two split
/// matches the pattern used by helix and other Rust TUI apps.
pub(crate) mod mouse;

/// Plugin surface: the `Plugin` trait + `PluginRegistry` + built-in
/// widget implementations (search, help, wiki) that dogfood the same
/// trait external plugins will use.
pub(crate) mod plugin;

/// File-based logging to XDG state directory.
pub mod logging;

/// Mapbox GL style JSON parser — filter compilation, color resolution.
pub mod styler;

// ── Internal modules (not part of the external surface) ──────────────────
//
// These are marked `pub` so integration tests and benchmarks under
// `tests/` / `benches/` — which are compiled as external crates — can
// reach them. `#[doc(hidden)]` signals "not stable API" to consumers.

#[doc(hidden)]
pub mod geo;
#[doc(hidden)]
pub mod palette;
#[doc(hidden)]
pub mod render;
#[doc(hidden)]
pub mod shared;
#[doc(hidden)]
pub mod tile;
pub(crate) mod ui;
