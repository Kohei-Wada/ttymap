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

/// **Prototype.** Helix-style compositor stack, candidate replacement
/// for `FocusManager` + `FocusSurface` + `Plugin`. Not wired in yet —
/// the file is a design reference + shape comparison (one converted
/// plugin `SearchComponent` lives at the bottom).
pub(crate) mod compositor;

/// Settings loaded from `~/.config/ttymap/config.toml` + CLI overrides.
pub mod config;

/// Map subsystem — viewport state, action dispatch, and the full map
/// rendering pipeline (tile fetch, styler, render thread). `MapFrame`
/// produced here is what the UI displays.
pub mod map;

/// Focus manager — single source of truth for "which surface owns the
/// keyboard". Sits above `ui/` because keyboard dispatch routes input
/// through it before falling back to global handlers.
pub(crate) mod focus;

/// Background responder — handles keys when no surface has focus
/// (Tab cycle, `:` palette, plugin activation, keymap fallback, gg).
/// Owned by `FocusManager`; peer of `CommandPalette` and
/// `PluginRegistry` in the focus model.
pub(crate) mod background;

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

/// Plugin surface: the `Plugin` trait + `PluginRegistry` + built-in
/// widget implementations (search, help, wiki) that dogfood the same
/// trait external plugins will use.
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
