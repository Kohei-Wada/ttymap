//! Map subsystem — domain state and the full map-rendering pipeline.
//!
//! `state.rs` / `action.rs` own the map viewport (center, zoom, running
//! flag) and the `Action` enum. The siblings are the implementation
//! machinery:
//!
//! - `tile/`    — MVT fetch + cache + decode
//! - `styler/`  — GL-style rules (dark / bright presets)
//! - `render/`  — render thread + pipeline + drawing primitives
//!
//! Everything map-specific lives under this module; the UI consumes
//! `MapFrame` (from `render`) without knowing how it was produced.

pub mod action;
pub mod api;
pub mod render;
pub mod state;
pub mod styler;
pub mod tile;

pub use action::Action;
pub use api::MapApi;
pub use state::{MapState, MapStateOptions, Viewport};
