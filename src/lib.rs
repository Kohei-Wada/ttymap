//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

// The crate is primarily a binary (see src/main.rs). The library exists so
// tests can share a module tree; external consumers only need the handful
// of items used by main.rs, so everything else is `pub(crate)`.

/// Application event loop and central message dispatcher. Also home
/// of the [`UserIntent`](app::UserIntent) vocabulary — the single
/// enum every emission site (palette, plugins, mouse, future RPC)
/// speaks and that [`app::App::dispatch`] interprets.
pub mod app;

/// Compositor — stack-based focus / modal system (helix-inspired).
/// Owns the `Vec<(CardId, Box<dyn Component>)>` stack, routes key
/// events to the focused component (with fall-through to BaseLayer),
/// orchestrates layout (Floating vs Sidebar), and surfaces the
/// `Component` / `Window` framework that plugin-side wrappers
/// (`LuaCardComponent`) implement. Top-level subsystem (peer of
/// `app/`, `lua/`, `input/`).
pub mod compositor;

/// Palette — `:`-triggered universal picker. Itself a [`Component`]
/// pushed onto the compositor stack; provider sub-modes (theme
/// picker, search, plugin commands) swap in place via
/// `PaletteAction::SwitchProvider`. Top-level peer rather than
/// frontend-internal because the Lua bridge implements
/// `PaletteProvider` directly.
pub mod palette;

/// CLI subcommand implementations. Each subcommand lives in its own
/// submodule; `main.rs` just parses the top-level enum and calls
/// [`commands::Command::run`].
pub mod commands;

/// Settings populated from `~/.config/ttymap/init.lua` + CLI overrides.
pub mod config;

/// Map subsystem — viewport state, action dispatch, and the full map
/// rendering pipeline (tile fetch, styler, render thread). `MapFrame`
/// produced here is what the UI displays.
pub mod map;

/// Theme — colour palette data (`ColorPalette`, `DARK`, `BRIGHT`) plus
/// the ratatui adapter (`UiTheme`). `ThemeId` drives everything. Lives
/// at the crate root because it's a **plugin-facing service** —
/// plugins read colours from it during `render()`, so putting it under
/// `ui/` would recreate the ui↔plugin cycle we just broke. Marked
/// `pub #[doc(hidden)]` so benches under `benches/` can reach
/// `ThemeId` without treating it as stable API.
#[doc(hidden)]
pub mod theme;

/// Input subsystem — raw-terminal-event ingest and translation
/// (input thread, keymap table, mouse adapter). Sits as a peer of
/// `map/` and `lua/`; the frontend pulls translated [`UserIntent`]s
/// out of it for each `AppEvent::Input`. `pub` so `main` can name
/// the [`input::thread::InputHandle`] it spawns at the composition
/// root.
pub mod input;

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
