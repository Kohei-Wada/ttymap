//! ttymap — terminal map viewer.
//!
//! Renders Mapbox Vector Tiles as Unicode Braille characters in the
//! terminal. Inspired by [mapscii](https://github.com/rastapasta/mapscii).
//!
//! `ttymap-app` is the **composition root**: it owns `App` (the
//! state hub + event-loop driver), `main` (CLI parse → subsystem
//! bootstrap → loop run), `EngineHandle` (TUI-side handle to the
//! `ttymap engine-worker` IPC subprocess), and XDG state logging.
//! Every other concern lives in a peer crate of the workspace —
//! see the top-level `CLAUDE.md` for the full layout.

/// Application — central state hub + event loop driver. Holds every
/// piece of mutable app-level state (map handle, lua handle,
/// compositor, theme, sidebar, …), drains the unified
/// [`app::AppEvent`] bus each iteration, and is the only place
/// `terminal.draw(...)` is called.
pub mod app;

/// `EngineHandle` — TUI-side handle to the `ttymap engine-worker`
/// subprocess. Wraps the parent end of the bincode-framed IPC
/// stream and presents the same surface as the in-process
/// `MapHandle` so [`app::App`] stays oblivious to the subprocess
/// split.
pub mod engine_handle;

/// File-based logging to XDG state directory.
pub mod logging;
