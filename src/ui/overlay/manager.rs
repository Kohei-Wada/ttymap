//! Aggregates every built-in [`MapOverlay`] into one object so callers
//! (app loop, dispatch, mouse handler, UI draw) don't need to know
//! which overlays are stateful, which have async work, or in what
//! order they paint. Adding a new overlay is one field + one line per
//! hook inside this module.

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::geo::LonLat;
use crate::map::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;

use super::{AttributionOverlay, InfoOverlay, MapOverlay, ScaleBarOverlay};

pub struct OverlayManager {
    info: InfoOverlay,
    attribution_text: Option<String>,
}

impl OverlayManager {
    pub fn new(nominatim: Arc<NominatimClient>, attribution: Option<String>) -> Self {
        Self {
            info: InfoOverlay::new(nominatim),
            attribution_text: attribution,
        }
    }

    /// Advance any overlay with async work by one tick.
    pub fn poll(&mut self) {
        self.info.poll();
    }

    /// Notify overlays whose content depends on the current view.
    pub fn on_map_moved(&mut self, center: LonLat) {
        self.info.on_map_moved(center);
    }

    /// Record the latest terminal cursor position from a mouse event.
    pub fn set_cursor(&mut self, pos: (u16, u16)) {
        self.info.set_cursor(pos);
    }

    /// Stamp every built-in overlay onto the buffer in paint order.
    pub fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &UiTheme) {
        let attribution = AttributionOverlay {
            text: self.attribution_text.as_deref().unwrap_or(""),
        };
        let overlays: [&dyn MapOverlay; 3] = [&self.info, &ScaleBarOverlay, &attribution];
        for o in overlays {
            o.render(buf, map_area, frame, theme);
        }
    }
}
