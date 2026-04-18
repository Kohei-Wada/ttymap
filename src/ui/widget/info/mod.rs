//! Info UI — coords/place and scale bar, both as map overlays.
//!
//! `InfoState` on `UiState` carries the display strings. `app.rs`
//! writes them; the two overlays read them and stamp on top of the map.

pub mod coords;
pub mod scale_bar;
pub mod state;

pub use coords::CoordsOverlay;
pub use scale_bar::ScaleBarOverlay;
pub use state::InfoState;
