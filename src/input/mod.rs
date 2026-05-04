//! Input subsystem — raw-terminal-event ingest and translation.
//!
//! Three sibling modules, all peers to render/tile/lua:
//!
//! - [`thread`] is the producer: a dedicated OS thread that blocks
//!   on `crossterm::event::read()` and pushes each event onto the
//!   App's unified queue as [`AppEvent::Input`](crate::app::AppEvent::Input).
//! - [`keymap`] holds the static [`KeyMap`] table + the
//!   [`KeybindingOverrides`] shape Lua's `keymap.set` populates.
//!   [`crate::app::App`] resolves a raw [`crossterm::event::KeyEvent`]
//!   against this table to produce an [`UserCommand`](crate::UserCommand).
//! - [`mouse`] holds the [`MouseAdapter`] that translates raw
//!   `crossterm::event::MouseEvent`s — drag, scroll, click — into
//!   `UserCommand`s using the current viewport for screen → world
//!   projection.
//!
//! The subsystem doesn't own state across frames; it's a pure
//! translation layer that the app pulls from. Keeping it separate
//! from `app/` makes the data flow obvious:
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

pub use keymap::{KeyMap, KeybindingOverrides};
pub use mouse::MouseAdapter;
