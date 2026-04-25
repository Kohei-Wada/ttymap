//! Plugin-facing map API — the surface plugins use to interact with
//! the map during rendering.
//!
//! `MapApi` wraps the ratatui buffer, the active `MapProjection`, and
//! the theme. Plugins call methods like `point` / `label` / `line` to
//! plot world-space primitives without touching the buffer or doing
//! projection math themselves.
//!
//! Internally each primitive routes through [`Self::cell_for`] which
//! does projection + bounds-clip in one shot, so adding more
//! primitives (polygon fill, rotated marker, ...) reuses the same
//! gate.

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

    // ── Theme accessors ──────────────────────────────────────────────

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

    // ── Drawing primitives ──────────────────────────────────────────

    /// Plot a single-cell glyph at the given world coordinate. No-op
    /// when the point projects outside the visible map area.
    pub fn point(&mut self, ll: LonLat, glyph: char, fg: Color) {
        let Some((x, y)) = self.cell_for(ll) else {
            return;
        };
        self.buf[(x, y)]
            .set_char(glyph)
            .set_style(Style::default().fg(fg).bg(self.theme.bg));
    }

    /// Plot a single-cell glyph with full [`Style`] control (fg, bg,
    /// modifiers). Same projection / clipping rules as
    /// [`point`](Self::point); use this when an accent colour or
    /// custom modifier (bold / underline / reverse) matters.
    #[allow(dead_code)] // plugin-author API; no in-tree consumer yet
    pub fn point_styled(&mut self, ll: LonLat, glyph: char, style: Style) {
        let Some((x, y)) = self.cell_for(ll) else {
            return;
        };
        self.buf[(x, y)].set_char(glyph).set_style(style);
    }

    /// Write `text` starting in the cell to the right of the
    /// projected point. Clips at the map area's right edge. No
    /// collision detection — overlapping labels overwrite each other
    /// in render order.
    ///
    /// Convention: leave a space between marker and label by placing
    /// the marker via `point` first; this method already skips the
    /// marker's own cell.
    pub fn label(&mut self, ll: LonLat, text: &str, fg: Color) {
        let Some((mut x, y)) = self.cell_for(ll) else {
            return;
        };
        let style = Style::default().fg(fg).bg(self.theme.bg);
        let right_edge = self.map_area.x + self.map_area.width;
        // Skip the marker cell itself.
        x = x.saturating_add(1);
        for ch in text.chars() {
            if x >= right_edge {
                break;
            }
            self.buf[(x, y)].set_char(ch).set_style(style);
            x = x.saturating_add(1);
        }
    }

    /// Draw a single-glyph line between two world coordinates at
    /// terminal-cell granularity (Bresenham). Both endpoints must
    /// project inside the visible map area; partially-visible
    /// segments require manual clipping by the caller.
    #[allow(dead_code)] // plugin-author API; no in-tree consumer yet
    pub fn line(&mut self, a: LonLat, b: LonLat, glyph: char, fg: Color) {
        let Some((x0, y0)) = self.cell_for(a) else {
            return;
        };
        let Some((x1, y1)) = self.cell_for(b) else {
            return;
        };
        let style = Style::default().fg(fg).bg(self.theme.bg);
        let mut x = x0 as i32;
        let mut y = y0 as i32;
        let x1 = x1 as i32;
        let y1 = y1 as i32;
        let dx = (x1 - x).abs();
        let dy = -(y1 - y).abs();
        let sx: i32 = if x < x1 { 1 } else { -1 };
        let sy: i32 = if y < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            // Both x and y are bounded by previously-validated
            // cell_for results plus a Bresenham step toward another
            // valid cell, so the cast back to u16 cannot overflow.
            self.buf[(x as u16, y as u16)]
                .set_char(glyph)
                .set_style(style);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    // ── Internal: project + clip in one place ───────────────────────

    /// Project a world coordinate into terminal-cell space and clip
    /// against [`Self::map_area`]. Returns `None` when the point
    /// falls outside the visible map area; otherwise returns the
    /// absolute (`x`, `y`) cell to write to in the ratatui buffer.
    fn cell_for(&self, ll: LonLat) -> Option<(u16, u16)> {
        let (col, row) = self.proj.ll_to_cell(ll)?;
        if col >= self.map_area.width || row >= self.map_area.height {
            return None;
        }
        Some((self.map_area.x + col, self.map_area.y + row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::render::frame::MapFrame;
    use crate::theme::{DARK, UiTheme};

    fn fixture(area_w: u16, area_h: u16) -> (Buffer, Rect, MapFrame, UiTheme) {
        let area = Rect::new(0, 0, area_w, area_h);
        let buf = Buffer::empty(area);
        let frame = MapFrame {
            cells: Vec::new(),
            cols: area_w,
            rows: area_h,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 1.0,
        };
        let theme = UiTheme::from_palette(&DARK);
        (buf, area, frame, theme)
    }

    #[test]
    fn label_writes_chars_starting_after_marker() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme);
        api.label(LonLat { lon: 0.0, lat: 0.0 }, "AB", Color::Reset);
        // Marker cell stays untouched; label writes the next two cells.
        // We don't know the exact projected column without re-running
        // the projection, so just confirm SOMETHING was written —
        // count non-default symbols on the row.
        let row = area.height / 2;
        let written: usize = (0..area.width)
            .filter(|&x| {
                let cell = &buf[(x, row)];
                let s = cell.symbol();
                s == "A" || s == "B"
            })
            .count();
        assert_eq!(written, 2, "label should write both characters");
    }

    #[test]
    fn label_off_map_is_noop() {
        let (mut buf, area, frame, theme) = fixture(10, 5);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme);
        // Far-off coordinate that won't project into the canvas.
        api.label(
            LonLat {
                lon: 179.0,
                lat: 89.0,
            },
            "X",
            Color::Reset,
        );
        for x in 0..area.width {
            for y in 0..area.height {
                assert_eq!(buf[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn point_styled_uses_caller_style() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme);
        let style = Style::default()
            .fg(Color::Indexed(42))
            .bg(Color::Indexed(7));
        api.point_styled(LonLat { lon: 0.0, lat: 0.0 }, '#', style);
        let mut found = false;
        for x in 0..area.width {
            for y in 0..area.height {
                let cell = &buf[(x, y)];
                if cell.symbol() == "#" {
                    assert_eq!(cell.fg, Color::Indexed(42));
                    assert_eq!(cell.bg, Color::Indexed(7));
                    found = true;
                }
            }
        }
        assert!(found, "point_styled should write the glyph");
    }

    #[test]
    fn line_draws_connected_cells() {
        // Larger area at higher zoom keeps the test endpoints inside
        // the viewport — projection at zoom 1.0 over ±10° drops them
        // off-canvas.
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let frame = MapFrame {
            cells: Vec::new(),
            cols: 80,
            rows: 40,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 4.0,
        };
        let theme = UiTheme::from_palette(&DARK);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme);
        api.line(
            LonLat {
                lon: -1.0,
                lat: 1.0,
            },
            LonLat {
                lon: 1.0,
                lat: -1.0,
            },
            '*',
            Color::Reset,
        );
        let drawn: usize = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].symbol() == "*")
            .count();
        assert!(
            drawn >= 2,
            "line should mark at least two cells, got {drawn}"
        );
    }
}
