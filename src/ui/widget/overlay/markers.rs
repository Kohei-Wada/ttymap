//! Generic markers overlay — stamps a glyph at each world-space point
//! in the supplied slice. Any domain (wiki, search results, future POI
//! types) that wants pins on the map builds a `Vec<MarkerPoint>` and
//! hands it to [`MarkersOverlay`]; no per-domain overlay impl needed.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::geo::{LonLat, MapProjection};
use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;

use super::MapOverlay;

#[derive(Debug, Clone, Copy)]
pub struct MarkerPoint {
    pub lon: f64,
    pub lat: f64,
    pub glyph: char,
    pub fg: Color,
}

pub struct MarkersOverlay<'a> {
    pub points: &'a [MarkerPoint],
}

impl MapOverlay for MarkersOverlay<'_> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme) {
        if self.points.is_empty() {
            return;
        }

        // Project against the full canvas the frame was rendered at, then
        // reject cells that would fall outside the area the map widget
        // actually painted onto (map_area may be smaller after a resize).
        let proj = MapProjection::new(frame.center, frame.zoom, frame.cols, frame.rows);

        for point in self.points {
            let Some((col, row)) = proj.ll_to_cell(LonLat {
                lon: point.lon,
                lat: point.lat,
            }) else {
                continue;
            };
            if col >= map_area.width || row >= map_area.height {
                continue;
            }
            buf[(map_area.x + col, map_area.y + row)]
                .set_char(point.glyph)
                .set_style(Style::default().fg(point.fg).bg(theme.bg));
        }
    }
}
