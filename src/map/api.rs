//! Plugin-facing map API — the surface plugins use to interact with
//! the map during rendering.
//!
//! `MapApi` wraps the ratatui buffer, the active `MapProjection`, and
//! the theme. Plugins call methods like `point` to plot world-space
//! primitives without touching the buffer or doing projection math
//! themselves. Adding more primitives (label, line, polygon) here
//! extends every plugin uniformly.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::geo::{LonLat, MapProjection};
use crate::map::render::frame::MapFrame;
use crate::theme::UiTheme;

pub struct MapApi<'a> {
    buf: &'a mut Buffer,
    map_area: Rect,
    proj: MapProjection,
    theme: &'a UiTheme,
}

impl<'a> MapApi<'a> {
    pub fn new(buf: &'a mut Buffer, map_area: Rect, frame: &MapFrame, theme: &'a UiTheme) -> Self {
        let proj = MapProjection::new(frame.center, frame.zoom, frame.cols, frame.rows);
        Self {
            buf,
            map_area,
            proj,
            theme,
        }
    }

    /// Primary accent colour — used by plugins to highlight features
    /// (wiki markers, search pins, ...). Semantic accessor; the
    /// underlying theme is hidden from plugins.
    pub fn accent_color(&self) -> Color {
        self.theme.accent
    }

    /// Secondary accent colour — typically used to distinguish the
    /// selected / focused feature from the rest.
    pub fn accent_alt_color(&self) -> Color {
        self.theme.accent_alt
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
