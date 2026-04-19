//! Top-right overlay — four info rows at fixed positions:
//!   row 0: map center lat/lon (always)
//!   row 1: mouse cursor lat/lon (or "unknown") — kept adjacent to
//!          row 0 so values can be visually diffed
//!   row 2: zoom level (always)
//!   row 3: reverse-geocoded place name (or "unknown")
//! Every row is always rendered so the readout never shifts; absent
//! values show "unknown" instead of leaving a gap.

use std::sync::Arc;
use std::time::Duration;

use log::debug;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::geo::{LonLat, MapProjection};
use crate::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;
use crate::shared::throttle::Throttle;
use crate::theme::UiTheme;

mod service;

use super::MapOverlay;
use service::{PlaceService, format_name};

pub struct InfoOverlay {
    cursor: Option<(u16, u16)>,
    place_name: Option<String>,
    place_service: PlaceService,
    place_throttle: Throttle,
}

impl InfoOverlay {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            cursor: None,
            place_name: None,
            place_service: PlaceService::new(nominatim),
            place_throttle: Throttle::ready(Duration::from_secs(5)),
        }
    }

    /// Record the latest terminal cursor position from a mouse event.
    pub fn set_cursor(&mut self, pos: (u16, u16)) {
        self.cursor = Some(pos);
    }

    /// Called by the app whenever the map recenters. Triggers a throttled
    /// reverse-geocode fetch.
    pub fn on_map_moved(&mut self, center: LonLat) {
        if self.place_throttle.check() {
            self.place_service.reverse(center);
        }
    }

    /// Drain any completed reverse-geocode results. Returns `true` if
    /// the displayed name changed (caller should redraw).
    pub fn poll(&mut self) -> bool {
        let Some(place) = self.place_service.poll() else {
            return false;
        };
        let new_name = place.map(format_name);
        if new_name != self.place_name {
            if let Some(ref name) = new_name {
                debug!("reverse: {}", name);
            }
            self.place_name = new_name;
            true
        } else {
            false
        }
    }
}

const CENTER_ROW: u16 = 0;
const CURSOR_ROW: u16 = 1;
const ZOOM_ROW: u16 = 2;
const PLACE_ROW: u16 = 3;

impl MapOverlay for InfoOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &UiTheme) {
        if map_area.width < 4 || map_area.height < 1 {
            return;
        }
        let style = Style::default().fg(theme.accent).bg(theme.bg);

        let center_line = format!(" center: {:.3}, {:.3}", frame.center.lat, frame.center.lon);
        draw_right_line(buf, map_area, CENTER_ROW, &center_line, style);

        if CURSOR_ROW < map_area.height {
            let line = match cursor_ll(self.cursor, map_area, frame) {
                Some(ll) => format!(" cursor: {:.3}, {:.3}", ll.lat, ll.lon),
                None => " cursor: unknown".to_string(),
            };
            draw_right_line(buf, map_area, CURSOR_ROW, &line, style);
        }

        if ZOOM_ROW < map_area.height {
            let line = format!(" zoom: {:.1}", frame.zoom);
            draw_right_line(buf, map_area, ZOOM_ROW, &line, style);
        }

        if PLACE_ROW < map_area.height {
            let name = self.place_name.as_deref().unwrap_or("unknown");
            let line = format!(" place: {}", name);
            draw_right_line(buf, map_area, PLACE_ROW, &line, style);
        }
    }
}

fn draw_right_line(buf: &mut Buffer, map_area: Rect, row_offset: u16, line: &str, style: Style) {
    let width = (line.width() as u16 + 2).min(map_area.width);
    let rect = Rect::new(
        map_area.right().saturating_sub(width),
        map_area.y + row_offset,
        width,
        1,
    );
    Clear.render(rect, buf);
    Paragraph::new(line.to_string())
        .style(style)
        .alignment(Alignment::Right)
        .render(rect, buf);
}

fn cursor_ll(cursor: Option<(u16, u16)>, map_area: Rect, frame: &MapFrame) -> Option<LonLat> {
    let (cx, cy) = cursor?;
    if cx < map_area.x || cy < map_area.y {
        return None;
    }
    let local_col = cx - map_area.x;
    let local_row = cy - map_area.y;
    MapProjection::new(frame.center, frame.zoom, frame.cols, frame.rows)
        .cell_to_ll(local_col, local_row)
}
