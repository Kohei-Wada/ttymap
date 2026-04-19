//! Input subsystem — keyboard and mouse dispatchers.
//!
//! Keyboard and mouse are **intentionally separate** here. They have
//! different semantics:
//!
//! - Keyboard events are **modal / captured**: a focused surface owns
//!   them, the keymap fallback chain resolves anything it passes. The
//!   dispatcher has to route through focus, Tab cycling, `:` palette,
//!   plugin activation, and finally map actions — five layers, each
//!   with its own concerns.
//! - Mouse events are **observer + target**: position hit-tests decide
//!   what they affect, and they always have a natural target (the
//!   map). The dispatcher just gates on modal widgets and forwards to
//!   map / info overlay.
//!
//! Collapsing the two into a single dispatcher has been a documented
//! regret in other Rust TUI apps (gitui). Keep them split.

pub mod keyboard;
pub mod mouse;
