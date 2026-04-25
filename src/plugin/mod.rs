//! Plugin modules.
//!
//! Under the compositor model, each plugin is a self-contained module
//! exposing `pub fn register(..., r: &mut Registrar)`. The `App` never
//! names a concrete plugin type; the composition root in `app/mod.rs`
//! calls each `register` in turn. See [`crate::compositor`] for the
//! Component / Painter / Task traits plugins implement.

pub mod aircraft;
pub mod export;
pub mod help;
pub mod here;
pub mod iss;
pub mod search;
pub mod wiki;
