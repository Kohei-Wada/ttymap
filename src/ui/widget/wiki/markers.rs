//! Wiki map overlay — `●` markers stamped on top of the rendered map,
//! with the selected article highlighted in `accent_alt`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::geo;
use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;
use crate::ui::widget::overlay::MapOverlay;

use super::state::WikiState;

/// Places a `●` on the map for each article. Selected one uses
/// `accent_alt` so it pops against the rest.
pub struct WikiMarkersOverlay<'a> {
    pub state: &'a WikiState,
}

impl MapOverlay for WikiMarkersOverlay<'_> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme) {
        let state = self.state;
        if !state.active || state.articles.is_empty() {
            return;
        }

        // Canvas size (pixels) that the frame was rendered at. Each terminal
        // cell is 2 pixels wide × 4 pixels tall under braille.
        let canvas_w = frame.cols as f64 * 2.0;
        let canvas_h = frame.rows as f64 * 4.0;

        let z = geo::base_zoom(frame.zoom);
        let tile_size = geo::tile_size_at_zoom(frame.zoom);
        let center_tile = geo::ll2tile(frame.center.lon, frame.center.lat, z);

        // Maximum cell coordinates within the map area we're allowed to
        // write into. Clamp to the frame size so we never draw beyond where
        // the map widget actually rendered.
        let max_col = frame.cols.min(map_area.width);
        let max_row = frame.rows.min(map_area.height);

        for (i, article) in state.articles.iter().enumerate() {
            let pt = geo::ll2tile(article.lon, article.lat, z);
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

            let fg = if i == state.selected {
                theme.accent_alt
            } else {
                theme.accent
            };
            let ch = '●';

            let x = map_area.x + cell_col;
            let y = map_area.y + cell_row;
            buf[(x, y)]
                .set_char(ch)
                .set_style(Style::default().fg(fg).bg(theme.bg));
        }
    }
}
