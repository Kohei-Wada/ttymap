//! Aggregates the remaining built-in [`MapOverlay`]s (scale_bar,
//! attribution) into one object so the UI draw site doesn't have to
//! know about each one.
//!
//! Info migrated to [`crate::plugin::info`]; this manager keeps
//! shrinking until scalebar and attribution follow, after which it
//! disappears entirely.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::map::render::frame::MapFrame;
use crate::theme::UiTheme;

use super::{AttributionOverlay, MapOverlay, ScaleBarOverlay};

pub struct OverlayManager {
    attribution_text: Option<String>,
}

impl OverlayManager {
    pub fn new(attribution: Option<String>) -> Self {
        Self {
            attribution_text: attribution,
        }
    }

    /// Stamp every built-in overlay onto the buffer in paint order.
    pub fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &UiTheme) {
        let attribution = AttributionOverlay {
            text: self.attribution_text.as_deref().unwrap_or(""),
        };
        let overlays: [&dyn MapOverlay; 2] = [&ScaleBarOverlay, &attribution];
        for o in overlays {
            o.render(buf, map_area, frame, theme);
        }
    }
}
