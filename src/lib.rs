//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

// The crate is primarily a binary (see src/main.rs). The library exists so
// tests can share a module tree; external consumers only need the handful
// of items used by main.rs, so everything else is `pub(crate)`.

/// Crate-wide command vocabulary — the GoF Command pattern's
/// **Command** role. The single enum every emission site (palette,
/// plugins, mouse, future RPC) speaks and that [`app::App::dispatch`]
/// interprets as the GoF Receiver. Foundational on purpose: every
/// layer that *produces* a command reaches this type via
/// `crate::UserCommand`, no upward dependency on `app/`.
pub mod command;
pub use command::UserCommand;

/// Engine layer — state that mutates in response to commands and
/// the GoF Receiver ([`core::Dispatcher`]) that runs them. Owned by
/// [`app::App`] but separated from it so the layering is visible
/// at the directory level. Ratatui-free.
pub mod core;

/// UI / IO shell layer. Houses presentation-bound modules: palette
/// picker, CLI subcommand entry. Sits above [`core`] and consumes
/// it; never imported from `core/`.
pub mod front;

/// Application event loop and central message dispatcher.
pub mod app;

/// Settings populated from `~/.config/ttymap/init.lua` + CLI overrides.
pub mod config;

/// Theme — colour palette data (`ColorPalette`, `DARK`, `BRIGHT`) plus
/// the ratatui adapter (`UiTheme`). `ThemeId` drives everything. Lives
/// at the crate root because it's a **plugin-facing service** —
/// plugins read colours from it during `render()`, so putting it under
/// `ui/` would recreate the ui↔plugin cycle we just broke. Marked
/// `pub #[doc(hidden)]` so benches under `benches/` can reach
/// `ThemeId` without treating it as stable API.
#[doc(hidden)]
pub mod theme;

/// File-based logging to XDG state directory.
pub mod logging;

/// Lua runtime for scripted plugins (mlua, Lua 5.4 vendored).
/// **Scaffold only** — owns the shared [`mlua::Lua`] state and the
/// bridge surface to `Component` / `PaletteProvider` / `MapApi`. No
/// production plugin uses this yet; expanded incrementally as the
/// fetch+render plugins migrate from Rust to Lua.
pub mod lua;

// ── Internal modules (not part of the external surface) ──────────────────
//
// These are marked `pub` so integration tests and benchmarks under
// `tests/` / `benches/` — which are compiled as external crates — can
// reach them. `#[doc(hidden)]` signals "not stable API" to consumers.

#[doc(hidden)]
pub mod geo;
#[doc(hidden)]
pub mod shared;
