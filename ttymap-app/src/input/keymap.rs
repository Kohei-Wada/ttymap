//! Re-export shim — the keymap types live in `ttymap-core`. This
//! file exists so existing `crate::input::keymap::*` imports keep
//! resolving; Phase B of the workspace split (moving `input/` into
//! `ttymap-tui`) will fix the call sites and remove this shim.

pub use ttymap_core::keymap::*;
