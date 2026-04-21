//! Input subsystem — pure device-event adapters.
//!
//! Modules here only translate raw crossterm events into a canonical
//! shape; focus-aware key routing lives one layer up in `ui::router`.
//! Mouse stays here because it carries real cross-event drag state
//! that is not focus-related (hit-test against the map widget).
//!
//! Keyboard and mouse are intentionally **not** unified into a single
//! dispatcher. They have different semantics:
//!
//! - Keyboard events are **modal / captured** — `ui::router` owns the
//!   focused-surface delivery + Tab cycling + `:` palette + plugin
//!   activation + keymap fallback chain.
//! - Mouse events are **observer + target** — position hit-tests
//!   decide what they affect, with a natural target (the map). The
//!   handler just gates on modal widgets and forwards to map / info
//!   overlay.
//!
//! Collapsing the two into a single dispatcher has been a documented
//! regret in other Rust TUI apps (gitui). Keep them split.

pub mod mouse;
