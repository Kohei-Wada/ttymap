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

// Crate-wide command vocabulary, event bus, configuration, and
// keymap types live in `ttymap-core` so the TUI / Lua / CLI crates
// can consume them without a circular dependency through `ttymap-app`.
// Re-exported as `crate::{command, event, config, UserCommand}` so
// existing call sites in this crate keep resolving.
pub use ttymap_core::{UserCommand, command};

/// Application — central state hub + event loop driver. Holds every
/// piece of mutable app-level state (map handle, lua handle,
/// compositor, theme, sidebar, …), drains the unified
/// [`app::AppEvent`] bus each iteration, and is the only place
/// `terminal.draw(...)` is called.
pub mod app;

/// `EngineHandle` — TUI-side handle to the `ttymap engine-worker`
/// subprocess. Wraps the parent end of the bincode-framed IPC stream
/// and presents the same surface as the in-process `MapHandle` so
/// [`app::App`] stays oblivious to the subprocess split.
pub mod engine_handle;

// Compositor, palette, theme, and input subsystems live in
// `ttymap-tui` so the Lua runtime and the future `ttymap-cli` crate
// can consume them without depending on `ttymap-app`. Re-exported
// here so existing `crate::compositor::*` / `crate::palette::*` /
// `crate::input::*` / `crate::theme::*` imports keep resolving until
// every consumer migrates to direct `ttymap_tui::*` use.
pub use ttymap_tui::{AppEvent, compositor, input, palette, theme};

// CLI subcommands live in `ttymap-cli` (`snap` + the
// `engine-worker` subprocess entry). Re-exported as `crate::cli`
// so `main.rs` keeps using `ttymap_app::cli::Command::run`.
pub use ttymap_cli as cli;

/// Settings populated from `~/.config/ttymap/init.lua` + CLI overrides.
/// Lives in `ttymap-core` for use by `ttymap-lua` (Lua bootstrap
/// reads `ttymap.opt.*` into this shape); re-exported here for
/// existing `crate::config::*` imports.
pub use ttymap_core::config;

// theme + input now live in `ttymap-tui` (re-exported above).

/// File-based logging to XDG state directory.
pub mod logging;

/// Pub/sub event subsystem — Lua-agnostic primitive at the
/// integration point between Rust core and Lua plugin runtime.
/// Lives in `ttymap-core`; re-exported here for existing
/// `crate::event::*` imports.
pub use ttymap_core::event;

// Lua runtime + bundled `runtime/` tree live in `ttymap-lua`.
// Re-exported as `crate::lua` so existing `crate::lua::*` call sites
// in `app/`, `main.rs`, and `cli/` keep resolving.
pub use ttymap_lua as lua;
