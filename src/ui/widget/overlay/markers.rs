//! Generic markers overlay — stamps a glyph at each world-space point
//! in the supplied slice. Any domain (wiki, search results, future POI
//! types) that wants pins on the map builds a `Vec<MarkerPoint>` and
//! hands it to [`MarkersOverlay`]; no per-domain overlay impl needed.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::geo;
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

        // Canvas size (pixels) the frame was rendered at. Each terminal
        // cell = 2 pixels wide × 4 pixels tall under braille.
        let canvas_w = frame.cols as f64 * 2.0;
        let canvas_h = frame.rows as f64 * 4.0;

        let z = geo::base_zoom(frame.zoom);
        let tile_size = geo::tile_size_at_zoom(frame.zoom);
        let center_tile = geo::ll2tile(frame.center.lon, frame.center.lat, z);

        // Clamp to the frame extent so we never draw beyond where the
        // map widget actually rendered.
        let max_col = frame.cols.min(map_area.width);
        let max_row = frame.rows.min(map_area.height);

        for point in self.points {
            let pt = geo::ll2tile(point.lon, point.lat, z);
            let px = canvas_w / 2.0 + (pt.x - center_tile.x) * tile_size;
            let py = canvas_h / 2.0 + (pt.y - center_tile.y) * tile_size;
            if !px.is_finite() || !py.is_finite() || px < 0.0 || py < 0.0 {
                continue;
            }

            let cell_col = (px / 2.0) as u16;
            let cell_row = (py / 4.0) as u16;
            if cell_col >= max_col || cell_row >= max_row {
                continue;
            }

            let x = map_area.x + cell_col;
            let y = map_area.y + cell_row;
            buf[(x, y)]
                .set_char(point.glyph)
                .set_style(Style::default().fg(point.fg).bg(theme.bg));
        }
    }
}
