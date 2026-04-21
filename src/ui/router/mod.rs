//! UI input routers — one per input device.
//!
//! - [`key::KeyRouter`] — keyboard routing (delivery to focused
//!   [`FocusSurface`](crate::focus::FocusSurface) + fall-through to
//!   background responder + `Effect::Open` focus transitions).
//! - [`mouse::MouseRouter`] — mouse event translation (drag-aware,
//!   emits `Ui(CursorMoved)` plus any pan/zoom command).
//!
//! The two paths intentionally do not share a dispatcher: keys are
//! modal/captured, mouse is observer+target, and unifying them has
//! been a regret in other Rust TUI apps (gitui). Both emit the same
//! `AppCommand` vocabulary on the output side, which is where the
//! symmetry is useful.

pub mod key;
pub mod mouse;
