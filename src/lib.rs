//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the terminal.
//! Inspired by [mapscii](https://github.com/rastapasta/mapscii).

// The crate is primarily a binary (see src/main.rs). The library exists so
// tests can share a module tree; external consumers only need the handful
// of items used by main.rs, so everything else is `pub(crate)`.

// Module organization is **flat by feature**, Neovim-inspired
// (cf. `src/nvim/`). The single concession to layering is `app/ui.rs`
// — the only place ratatui's `terminal.draw(...)` is called — which
// stays inside `app/` rather than getting its own `tui/` subdir.
// Everything else (compositor / map / input / palette / theme / lua)
// is a peer subsystem, named for what it does rather than for which
// layer it's "supposed" to belong to. We tried a strict `core/front`
// split (issue #212 Phase 4) and found it forced too many exceptional
// cases — sidebar policy is "UI" but lived in core because dispatcher
// owns it, theme_id leaked into core because every command tracked it,
// etc. Flat sidesteps all that.

/// Crate-wide command vocabulary — the GoF Command pattern's
/// **Command** role. The single enum every emission site (palette,
/// plugins, mouse, future RPC) speaks and that [`app::App::dispatch`]
/// interprets as the GoF Receiver. Sits at the crate root so every
/// emission site reaches it via `crate::UserCommand`.
pub mod command;
pub use command::UserCommand;

/// Application event loop, dispatch, and ratatui draw entry.
/// `App` owns [`app::dispatcher::Dispatcher`] (the GoF Receiver),
/// drains the unified [`app::AppEvent`] bus each iteration, and is
/// the only place `terminal.draw(...)` is called.
pub mod app;

/// Compositor — stack-based focus / modal system (helix-inspired).
/// Owns the `Vec<(CardId, Box<dyn Component>)>` stack, routes key
/// events to the focused component, and exposes the `Component` /
/// `Window` framework that plugin-side wrappers (`LuaCardComponent`)
/// implement.
pub mod compositor;

/// Palette — `:`-triggered universal picker. Itself a [`Component`]
/// pushed onto the compositor stack; provider sub-modes (theme
/// picker, search, plugin commands) swap in place via
/// `PaletteAction::SwitchProvider`.
pub mod palette;

/// CLI subcommand implementations. Each subcommand lives in its own
/// submodule; `main.rs` parses the top-level enum and calls
/// [`cli::Command::run`].
pub mod cli;

/// Settings populated from `~/.config/ttymap/init.lua` + CLI overrides.
pub mod config;

/// Map subsystem — viewport state, action dispatch, and the full map
/// rendering pipeline (tile fetch, styler, render thread). `MapFrame`
/// produced here is what the UI displays.
pub mod map;

/// Theme — colour palette data (`ColorPalette`, `DARK`, `BRIGHT`),
/// `ThemeId`, plus the ratatui adapter (`UiTheme`) and semantic-tag
/// resolver (`StyleKind`). All in one place because separating data
/// from adapter created more boilerplate than it saved.
#[doc(hidden)]
pub mod theme;

/// Input subsystem — raw-terminal-event ingest and translation
/// (input thread, keymap table, mouse adapter). [`app::App`] pulls
/// translated [`UserCommand`]s out of it for each `AppEvent::Input`.
pub mod input;

/// File-based logging to XDG state directory.
pub mod logging;

/// Lua runtime for scripted plugins (mlua, Lua 5.4 vendored). The
/// bridge lives here: api/ exposes the `ttymap` global to scripts,
/// bridge/ adapts Lua specs to Rust traits (`Component`,
/// `PaletteProvider`).
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
