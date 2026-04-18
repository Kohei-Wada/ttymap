//! Wiki UI — side panel (list/detail) plus a map marker overlay.
//!
//! State lives in [`WikiState`] on `UiState`. Both the panel renderer
//! ([`render_panel`]) and the map overlay ([`WikiMarkersOverlay`])
//! borrow this state, so they always agree on what is selected.

pub mod markers;
pub mod panel;
pub mod state;

pub use markers::WikiMarkersOverlay;
pub use panel::render_panel;
pub use state::{WikiAction, WikiState};
