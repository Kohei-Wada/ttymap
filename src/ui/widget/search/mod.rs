//! Search UI — center popup for forward geocoding (input + candidates).
//!
//! State lives in [`SearchState`] on `UiState`; `app.rs` owns mutation
//! (open, set_candidates, handle_key). The panel renderer only reads.

pub mod panel;
pub mod state;

pub use panel::render_panel;
pub use state::{SearchAction, SearchState};
