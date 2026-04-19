//! Built-in map overlays — part of the map-viewer identity, not
//! plugin territory. Info, attribution, and scale-bar are always on
//! screen; they implement [`MapOverlay`] and stamp themselves onto the
//! ratatui buffer after the base map.
//!
//! World-space primitives contributed by widgets (e.g. wiki markers)
//! go through [`crate::painter::MapPainter`], not an overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::frame::MapFrame;
use crate::theme::UiTheme;

pub mod attribution;
pub mod info;
pub mod scale_bar;

pub use attribution::AttributionOverlay;
pub use info::InfoOverlay;
pub use scale_bar::ScaleBarOverlay;

/// A drawable layer stamped on top of the rendered map.
///
/// `map_area` is the terminal rect occupied by the map. `frame` carries
/// the center/zoom/dimensions the map was rendered at so overlays can
/// project world coordinates back to screen cells.
pub trait MapOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &UiTheme);
}
