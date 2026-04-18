//! Top-right overlay — four info rows at fixed positions:
//!   row 0: map center lat/lon (always)
//!   row 1: mouse cursor lat/lon (or "unknown") — kept adjacent to
//!          row 0 so values can be visually diffed
//!   row 2: zoom level (always)
//!   row 3: reverse-geocoded place name (or "unknown")
//! Every row is always rendered so the readout never shifts; absent
//! values show "unknown" instead of leaving a gap.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use log::debug;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::geo::{self, LonLat};
use crate::render::frame::MapFrame;
use crate::shared::nominatim::{NominatimClient, PlaceInfo};
use crate::shared::throttle::Throttle;
use crate::ui::theme::Theme;

use super::MapOverlay;

pub struct InfoWidget {
    cursor: Option<(u16, u16)>,
    place_name: Option<String>,
    place_service: PlaceService,
    place_throttle: Throttle,
}

impl InfoWidget {
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

impl MapOverlay for InfoWidget {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme) {
        if map_area.width < 4 || map_area.height < 1 {
            return;
        }
        let style = Style::default().fg(theme.accent).bg(theme.bg);

        let center_line = format!(" center: {:.3}, {:.3}", frame.center.lat, frame.center.lon);
        draw_right_line(buf, map_area, CENTER_ROW, &center_line, style);

        if ZOOM_ROW < map_area.height {
            let line = format!(" zoom: {:.1}", frame.zoom);
            draw_right_line(buf, map_area, ZOOM_ROW, &line, style);
        }

        if CURSOR_ROW < map_area.height {
            let cursor_ll = self
                .cursor
                .and_then(|pos| cursor_to_ll(pos, map_area, frame));
            let line = match cursor_ll {
                Some(ll) => format!(" cursor: {:.3}, {:.3}", ll.lat, ll.lon),
                None => " cursor: unknown".to_string(),
            };
            draw_right_line(buf, map_area, CURSOR_ROW, &line, style);
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

fn cursor_to_ll(cursor: (u16, u16), map_area: Rect, frame: &MapFrame) -> Option<LonLat> {
    let (cx, cy) = cursor;
    if cx < map_area.x || cy < map_area.y {
        return None;
    }
    let cell_col = cx - map_area.x;
    let cell_row = cy - map_area.y;
    if cell_col >= frame.cols || cell_row >= frame.rows {
        return None;
    }

    let px = cell_col as f64 * 2.0 + 1.0;
    let py = cell_row as f64 * 4.0 + 2.0;
    let canvas_w = frame.cols as f64 * 2.0;
    let canvas_h = frame.rows as f64 * 4.0;

    let z = geo::base_zoom(frame.zoom);
    let tile_size = geo::tile_size_at_zoom(frame.zoom);
    let center_tile = geo::ll2tile(frame.center.lon, frame.center.lat, z);

    let tx = center_tile.x + (px - canvas_w / 2.0) / tile_size;
    let ty = center_tile.y + (py - canvas_h / 2.0) / tile_size;
    Some(geo::tile2ll(tx, ty, z))
}

// ── Service: async wrapper for reverse geocoding ─────────────────────────────

struct PlaceService {
    client: Arc<NominatimClient>,
    tx: mpsc::Sender<Option<PlaceInfo>>,
    rx: mpsc::Receiver<Option<PlaceInfo>>,
}

impl PlaceService {
    fn new(client: Arc<NominatimClient>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx }
    }

    fn reverse(&self, center: LonLat) {
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let result = client.reverse(center.lat, center.lon);
            let _ = tx.send(result);
        });
    }

    fn poll(&self) -> Option<Option<PlaceInfo>> {
        self.rx.try_recv().ok()
    }
}

fn format_name(place: PlaceInfo) -> String {
    match (place.city, place.country) {
        (Some(city), Some(country)) => format!("{}, {}", city, country),
        (None, Some(country)) => country,
        (Some(city), None) => city,
        (None, None) => place.display_name,
    }
}
