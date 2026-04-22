//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

// The crate is primarily a binary (see src/main.rs). The library exists so
// tests can share a module tree; external consumers only need the handful
// of items used by main.rs, so everything else is `pub(crate)`.

/// Application event loop and central message dispatcher. Also home
/// of the [`AppMsg`](app::AppMsg) vocabulary — the single enum every
/// emission site (palette, plugins, mouse, future RPC) speaks and that
/// [`App::dispatch`](app::App) interprets.
pub mod app;

/// CLI subcommand implementations. Each subcommand lives in its own
/// submodule; `main.rs` just parses the top-level enum and calls
/// [`commands::Command::run`].
pub mod commands;

/// Helix-style compositor stack — the focus/modal system that
/// replaced the old FocusManager / FocusSurface / Plugin trio. Holds
/// the `Component`, `Painter`, `Task`, `Registrar` abstractions that
/// every plugin plugs into, plus the always-on `BaseLayer`
/// (keymap fallback + activation dispatch).
pub(crate) mod compositor;

/// Settings loaded from `~/.config/ttymap/config.toml` + CLI overrides.
pub mod config;

/// Map subsystem — viewport state, action dispatch, and the full map
/// rendering pipeline (tile fetch, styler, render thread). `MapFrame`
/// produced here is what the UI displays.
pub mod map;

/// UI color set (Theme) + runtime theme-switch helper. Lives at the
/// crate root because it's a **plugin-facing service** — plugins read
/// colors from it during `render()`, so putting it under `ui/` would
/// recreate the ui↔plugin cycle we just broke.
pub(crate) mod theme;

/// `MapPainter` — world-space drawing primitives plugins use inside
/// `paint_on_map`. Also plugin-facing, also lives at the crate root
/// for the same cycle-avoidance reason as `theme`.
pub(crate) mod painter;

/// Key binding table and TOML override shape.
pub(crate) mod keymap;

/// Plugin modules. Each plugin exposes a `pub fn register(...,
/// &mut Registrar)` that plugs it into the compositor / painters /
/// tasks / palette. `App` is plugin-agnostic — only
/// `build_registrar` in `app/mod.rs` names plugins by module path.
pub(crate) mod plugin;

/// File-based logging to XDG state directory.
pub mod logging;

// ── Internal modules (not part of the external surface) ──────────────────
//
// These are marked `pub` so integration tests and benchmarks under
// `tests/` / `benches/` — which are compiled as external crates — can
// reach them. `#[doc(hidden)]` signals "not stable API" to consumers.

#[doc(hidden)]
pub mod color_palette;
#[doc(hidden)]
pub mod geo;
#[doc(hidden)]
pub mod shared;
pub(crate) mod ui;
