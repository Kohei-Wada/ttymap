//! Map overlay layer abstraction and the concrete pure-overlay widgets.
//!
//! The map widget renders the base map and stops. Anything drawn on top —
//! wiki markers, scale bar, future route/traffic layers — implements
//! [`MapOverlay`] and gets stamped onto the same buffer after the map in
//! the layout pass. Adding a new overlay means implementing the trait,
//! not touching the map widget. Designed like Google Maps' layer stack:
//! base map + independently toggle-able overlays.
//!
//! Hybrid widgets that also expose a panel (e.g. `wiki`) keep their
//! overlay impl in their own directory since it shares state with the
//! panel; only *pure* overlays live here.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;

pub mod coords;
pub mod place;
pub mod scale_bar;

pub use coords::CoordsOverlay;
pub use place::{PlaceOverlay, PlaceState};
pub use scale_bar::ScaleBarOverlay;

/// A drawable layer stamped on top of the rendered map.
///
/// `map_area` is the terminal rect occupied by the map. `frame` carries
/// the center/zoom/dimensions the map was rendered at so overlays can
/// project world coordinates back to screen cells.
pub trait MapOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme);
}
