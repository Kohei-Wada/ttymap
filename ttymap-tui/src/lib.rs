//! ttymap-tui — UI primitives that sit between `ttymap-engine` (the
//! rendering engine) and `ttymap-app` (the composition root / main
//! binary).
//!
//! Owns:
//!
//! - [`compositor`] — stack-based focus / modal system: `BaseLayer`,
//!   `Component`, `Window`, `Op`, focus stack render.
//! - [`palette`] — `:`-triggered universal picker, palette providers
//!   (theme / command / debounced async).
//! - [`theme`] — ratatui-side adapter (`UiTheme`, `StyleKind`)
//!   wrapping the engine's `ColorPalette` data.
//! - [`input`] — raw terminal event ingest + translation: keymap
//!   table, mouse adapter, input thread.
//! - [`app_event`] — `AppEvent` enum drained by the App main loop.
//!
//! Depends on `ttymap-engine` (renderer / theme palette / geo types),
//! `ttymap-core` (UserCommand vocabulary, EventBus), and
//! `ttymap-config` (KeybindingOverrides — the user-facing keymap
//! settings shape `KeyMap::with_overrides` folds in) — not on
//! `ttymap-lua` or `ttymap-app`. The plugin runtime reaches the UI
//! primitives through the trait surfaces exposed here
//! (`ActivationIndex`, `PaletteIndex`, `Component`, …).

pub mod app_event;
pub mod compositor;
pub mod frame_widget;
pub mod input;
pub mod overlay;
pub mod palette;
pub mod sidebar;
pub mod theme;

pub use app_event::AppEvent;
