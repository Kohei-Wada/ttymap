//! Drawing API handed to widgets during the paint phase.
//!
//! `MapPainter` wraps the ratatui buffer, the active `MapProjection`,
//! and the theme. Widgets call methods like `point` to plot world-space
//! primitives without touching the buffer or doing projection math
//! themselves. Adding more primitives (label, line, polygon) here
//! extends every widget uniformly.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::geo::{LonLat, MapProjection};
use crate::render::frame::MapFrame;
use crate::theme::UiTheme;

pub struct MapPainter<'a> {
    buf: &'a mut Buffer,
    map_area: Rect,
    proj: MapProjection,
    theme: &'a UiTheme,
}

impl<'a> MapPainter<'a> {
    pub fn new(buf: &'a mut Buffer, map_area: Rect, frame: &MapFrame, theme: &'a UiTheme) -> Self {
        let proj = MapProjection::new(frame.center, frame.zoom, frame.cols, frame.rows);
        Self {
            buf,
            map_area,
            proj,
            theme,
        }
    }

    pub fn theme(&self) -> &UiTheme {
        self.theme
    }

    /// Plot a single-cell glyph at the given world coordinate. No-op
    /// when the point projects outside the visible map area.
    pub fn point(&mut self, ll: LonLat, glyph: char, fg: Color) {
        let Some((col, row)) = self.proj.ll_to_cell(ll) else {
            return;
        };
        if col >= self.map_area.width || row >= self.map_area.height {
            return;
        }
        self.buf[(self.map_area.x + col, self.map_area.y + row)]
            .set_char(glyph)
            .set_style(Style::default().fg(fg).bg(self.theme.bg));
    }
}
