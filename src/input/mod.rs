//! Input subsystem — raw-terminal-event ingest and translation.
//!
//! Three sibling modules, all peers to render/tile/lua:
//!
//! - [`thread`] is the producer: a dedicated OS thread that blocks
//!   on `crossterm::event::read()` and pushes each event onto the
//!   App's unified queue as [`AppEvent::Input`](crate::frontend::AppEvent::Input).
//! - [`keymap`] holds the static [`KeyMap`] table + the
//!   [`KeybindingOverrides`] shape Lua's `keymap.set` populates.
//!   The frontend resolves a raw [`crossterm::event::KeyEvent`]
//!   against this table to produce an [`UserIntent`](crate::frontend::UserIntent).
//! - [`mouse`] holds the [`MouseAdapter`] that translates raw
//!   `crossterm::event::MouseEvent`s — drag, scroll, click — into
//!   `UserIntent`s using the current viewport for screen → world
//!   projection.
//!
//! The subsystem doesn't own state across frames; it's a pure
//! translation layer that the frontend pulls from. Keeping it
//! separate from `frontend/` makes the data flow obvious:
//!
//! ```text
//! crossterm  ─►  input::thread  ─►  AppEvent::Input
//!                                          │
//!                                          ▼
//!                                  frontend::handle_event
//!                                          │
//!                            ┌─────────────┴─────────────┐
//!                            ▼                           ▼
//!                       input::KeyMap                input::MouseAdapter
//!                            │                           │
//!                            └────────► UserIntent ◄─────────┘
//! ```

pub mod keymap;
pub mod mouse;
pub mod thread;

pub use keymap::{KeyMap, KeybindingOverrides};
pub use mouse::MouseAdapter;
