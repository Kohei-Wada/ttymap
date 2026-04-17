//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

/// Application event loop and terminal I/O orchestration.
pub mod app;

/// Color conversion utilities (hex → RGB → xterm-256).
pub mod color;

/// Core state management — config, input, markers, map state snapshots.
pub mod core;

/// Nominatim geocoding — place name to coordinates.
pub mod geocode;

/// Nominatim API client — forward and reverse geocoding.
pub mod nominatim;

/// Geographic coordinate math — Web Mercator projection, distance calculation.
pub mod geo;

/// File-based logging to XDG state directory.
pub mod logging;

/// Centralized color palette — xterm-256 indices for all themes.
pub mod palette;

/// Rendering pipeline — Braille buffer, canvas, clipping, render thread.
pub mod render;

/// Mapbox GL style JSON parser — filter compilation, color resolution.
pub mod styler;

/// Tile subsystem — cache, HTTP client, protobuf decode, view calculations.
pub mod tile;

/// Wikipedia API client — geosearch and page summaries.
pub mod wikipedia;

/// UI layout and widget rendering for ratatui.
pub mod ui;
