//! Input subsystem — raw-terminal-event ingest and translation.
//!
//! Three sibling modules:
//!
//! - [`thread`] is the producer: a dedicated OS thread that blocks
//!   on `crossterm::event::read()` and pushes each event onto the
//!   App's unified queue as
//!   [`AppEvent::Input`](crate::app_event::AppEvent::Input).
//! - [`keymap`] holds the static [`KeyMap`] table.
//!   [`KeybindingOverrides`](ttymap_config::KeybindingOverrides),
//!   the user-facing override shape that Lua's `keymap.set`
//!   populates, lives in `ttymap-config` (it's a setting);
//!   `KeyMap::with_overrides` folds the override map into a
//!   live binding table. The binary resolves raw
//!   `crossterm::event::KeyEvent`s against this table to produce
//!   a [`UserCommand`](ttymap_core::UserCommand).
//! - [`mouse`] holds the [`MouseAdapter`] that translates raw
//!   `crossterm::event::MouseEvent`s — drag, scroll, click — into
//!   `UserCommand`s using the current viewport for screen → world
//!   projection.
//!
//! The subsystem doesn't own state across frames; it's a pure
//! translation layer that the binary pulls from. The data flow:
//!
//! ```text
//! crossterm  ─►  input::thread  ─►  AppEvent::Input
//!                                          │
//!                                          ▼
//!                                    App::handle_event
//!                                          │
//!                            ┌─────────────┴─────────────┐
//!                            ▼                           ▼
//!                       input::KeyMap                input::MouseAdapter
//!                            │                           │
//!                            └────────► UserCommand ◄─────────┘
//! ```

pub mod keymap;
pub mod mouse;
pub mod thread;

pub use keymap::KeyMap;
pub use mouse::MouseAdapter;
pub use ttymap_config::KeybindingOverrides;
