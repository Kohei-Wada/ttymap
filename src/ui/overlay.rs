//! Map overlay layer abstraction.
//!
//! The map widget renders the base map and stops. Anything drawn on top —
//! wiki markers, scale bar, future route/traffic layers — implements
//! [`MapOverlay`] and gets stamped onto the same buffer after the map in
//! the layout pass. Adding a new overlay means implementing the trait,
//! not touching the map widget. Designed like Google Maps' layer stack:
//! base map + independently toggle-able overlays.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;

/// A drawable layer stamped on top of the rendered map.
///
/// `map_area` is the terminal rect occupied by the map. `frame` carries
/// the center/zoom/dimensions the map was rendered at so overlays can
/// project world coordinates back to screen cells.
pub trait MapOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme);
}
