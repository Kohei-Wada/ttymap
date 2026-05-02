//! Plugin-facing map API — the surface plugins use to interact with
//! the map during rendering.
//!
//! `MapApi` wraps the ratatui buffer, the active `MapProjection`, and
//! the theme. Two flavours of primitive:
//!
//! - **World-space** (`point`, `label`, ...) — input is a `LonLat`;
//!   projection + clipping handled internally.
//! - **Screen-space** (`text_anchored`, `cursor_ll`, ...) — input is
//!   anchored to a corner of the visible map area; useful for chrome
//!   overlays (info bar, scale, attribution) that don't track world
//!   coordinates.
//!
//! Internally world primitives route through [`Self::cell_for`]
//! which does projection + bounds-clip in one shot, so adding more
//! primitives (polygon fill, rotated marker, ...) reuses the same
//! gate.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::geo::{LonLat, MapProjection};
use crate::map::render::frame::MapFrame;
use crate::map::render::overlay::UserPolyline;
use crate::theme::UiTheme;

/// Corner-anchor for screen-space primitives like
/// [`MapApi::text_anchored`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // plugin-author API; in-tree consumers (info / scalebar / attribution plugins) land later
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub struct MapApi<'a> {
    buf: &'a mut Buffer,
    map_area: Rect,
    proj: MapProjection,
    theme: &'a UiTheme,
    /// Centre of the rendered frame this API draws over. Distinct
    /// from `App`'s current `MapState::center` because the frame is
    /// a snapshot the render thread already produced — chrome
    /// (info bar) shows the displayed view's centre, not the
    /// in-flight target.
    frame_center: LonLat,
    /// Zoom level baked into the rendered frame, same caveat as
    /// `frame_center`.
    frame_zoom: f64,
    /// Latest mouse cursor in absolute terminal cells. Surfaced
    /// here (in addition to `Context.cursor`) so paint_on_map can
    /// reach it without the plugin stashing a copy in its own state.
    cursor: Option<(u16, u16)>,
    /// Polylines pushed by Lua plugins / components this frame.
    /// Drained by `App` after `ui::draw` returns and bundled into the
    /// next `RenderTask::Draw`.
    overlay_sink: &'a mut Vec<UserPolyline>,
}

impl<'a> MapApi<'a> {
    pub fn new(
        buf: &'a mut Buffer,
        map_area: Rect,
        frame: &MapFrame,
        theme: &'a UiTheme,
        cursor: Option<(u16, u16)>,
        overlay_sink: &'a mut Vec<UserPolyline>,
    ) -> Self {
        let proj = MapProjection::new(frame.center, frame.zoom, frame.cols, frame.rows);
        Self {
            buf,
            map_area,
            proj,
            theme,
            frame_center: frame.center,
            frame_zoom: frame.zoom,
            cursor,
            overlay_sink,
        }
    }

    /// Centre coordinate of the rendered map snapshot — what the
    /// user is actually looking at this frame.
    pub fn center(&self) -> LonLat {
        self.frame_center
    }

    /// Zoom level baked into the rendered map snapshot.
    pub fn zoom(&self) -> f64 {
        self.frame_zoom
    }

    /// Mouse cursor projected to world coordinates, or `None` when
    /// the cursor is outside the map area or no mouse event has
    /// arrived yet. Convenience over [`cursor_ll`](Self::cursor_ll)
    /// — the cursor is already known to the API.
    pub fn cursor(&self) -> Option<LonLat> {
        self.cursor_ll(self.cursor?)
    }

    /// Width of the visible map area in terminal cells. Useful for
    /// chrome that scales with the canvas (e.g. the scale bar).
    pub fn area_width(&self) -> u16 {
        self.map_area.width
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

    /// Muted foreground for chrome that should fade into the
    /// background — attribution text, less-important readouts.
    pub fn muted_color(&self) -> Color {
        self.theme.muted_color
    }

    /// Active palette's "motorway" road colour as an xterm-256 index.
    /// Surfaced for the Lua bridge so plugins can request the same
    /// colour the map uses for major roads — useful for overlay
    /// polylines that should blend visually with the map's existing
    /// line features.
    pub fn road_color_xterm(&self) -> u8 {
        self.theme.palette.road_motorway
    }

    /// Active palette's `accent` colour as an xterm-256 index. Useful
    /// for plugins that want to feed the same colour into `map:polyline`
    /// or other APIs that take xterm indices.
    pub fn accent_color_xterm(&self) -> u8 {
        match self.accent_color() {
            ratatui::style::Color::Indexed(i) => i,
            _ => 7,
        }
    }

    /// Active palette's `accent_alt` colour as an xterm-256 index.
    pub fn accent_alt_color_xterm(&self) -> u8 {
        match self.accent_alt_color() {
            ratatui::style::Color::Indexed(i) => i,
            _ => 7,
        }
    }

    /// Active palette's `muted` colour as an xterm-256 index.
    pub fn muted_color_xterm(&self) -> u8 {
        match self.muted_color() {
            ratatui::style::Color::Indexed(i) => i,
            _ => 7,
        }
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

    /// Queue a subpixel polyline for the next render frame. The line is
    /// drawn by the render thread on the same `BrailleBuffer` that holds
    /// tile features, so dots OR-merge with existing tile pixels and the
    /// overlay's `fg` wins where they overlap (`BrailleBuffer::set_pixel`
    /// does `pixel_buf |= bit; fg_buf = color`).
    ///
    /// Coords with fewer than 2 points are silently dropped — see the
    /// Lua-bridge filter in `src/lua/ttymap/map_api.rs::polyline`.
    /// Off-canvas portions of the projected line are no-ops (Bresenham
    /// + `set_pixel` bounds check).
    pub fn push_polyline_overlay(&mut self, coords: Vec<LonLat>, color: u8) {
        self.overlay_sink.push(UserPolyline { coords, color });
    }

    /// Write `text` starting in the cell to the right of the
    /// projected point. Clips at the map area's right edge. No
    /// collision detection — overlapping labels overwrite each other
    /// in render order.
    ///
    /// Convention: leave a space between marker and label by placing
    /// the marker via `point` first; this method already skips the
    /// marker's own cell.
    ///
    /// CJK wide chars are placed correctly (each occupies two cells)
    /// because the underlying `set_stringn` consults
    /// `UnicodeWidthStr` rather than counting code points.
    pub fn label(&mut self, ll: LonLat, text: &str, fg: Color) {
        let Some((x, y)) = self.cell_for(ll) else {
            return;
        };
        let style = Style::default().fg(fg).bg(self.theme.bg);
        let right_edge = self.map_area.x + self.map_area.width;
        // Skip the marker cell itself.
        let label_start = x.saturating_add(1);
        if label_start >= right_edge {
            return;
        }
        let max_cells = (right_edge - label_start) as usize;
        // Truncate text to what fits in the remaining cells; the
        // truncation respects character boundaries (no partial CJK
        // half-cells written).
        let mut budget = max_cells;
        let mut end = 0;
        for (i, ch) in text.char_indices() {
            let w = ch.width().unwrap_or(0);
            if w > budget {
                break;
            }
            budget -= w;
            end = i + ch.len_utf8();
        }
        self.buf
            .set_stringn(label_start, y, &text[..end], max_cells, style);
    }

    // ── Screen-space primitives ─────────────────────────────────────

    /// Write a single line of `text` anchored to a corner of the map
    /// area, offset `rows_in` rows from that corner toward the
    /// interior. Right-side anchors right-align; left-side anchors
    /// left-align. Truncated when the map area is too narrow.
    /// Background uses the theme's `bg` so the overlay reads against
    /// the rendered map.
    ///
    /// Used by chrome overlays (info bar in the top-right, scale bar
    /// in the bottom-left, attribution in the bottom-right) that
    /// don't track world coordinates.
    #[allow(dead_code)] // plugin-author API; in-tree consumers land in later steps
    pub fn text_anchored(&mut self, anchor: Anchor, rows_in: u16, text: &str, fg: Color) {
        if self.map_area.width == 0 || self.map_area.height == 0 {
            return;
        }
        if rows_in >= self.map_area.height {
            return;
        }

        let row = match anchor {
            Anchor::TopLeft | Anchor::TopRight => self.map_area.y + rows_in,
            Anchor::BottomLeft | Anchor::BottomRight => {
                self.map_area.y + self.map_area.height - 1 - rows_in
            }
        };

        let area_width = self.map_area.width;
        // `UnicodeWidthStr::width` reports display columns (CJK wide
        // chars count as 2), unlike `chars().count()` which counts
        // code points. Right-aligned anchors need the visual width so
        // a Japanese place name doesn't slide off the right edge.
        let text_width = (text.width() as u16).min(area_width);

        let start_x = match anchor {
            Anchor::TopLeft | Anchor::BottomLeft => self.map_area.x,
            Anchor::TopRight | Anchor::BottomRight => {
                let right = self.map_area.x + area_width;
                right.saturating_sub(text_width)
            }
        };

        let style = Style::default().fg(fg).bg(self.theme.bg);
        // Buffer::set_stringn handles wide-char placement correctly:
        // a CJK char occupies cell N and marks N+1 as continuation;
        // an English char occupies one cell.
        self.buf
            .set_stringn(start_x, row, text, text_width as usize, style);
    }

    /// Project an absolute terminal cursor position (as surfaced via
    /// [`Context::cursor`](crate::frontend::compositor::Context::cursor)) into
    /// world coordinates. Returns `None` when the cursor is outside
    /// the visible map area.
    #[allow(dead_code)] // plugin-author API; in-tree consumer (info plugin) lands later
    pub fn cursor_ll(&self, cursor: (u16, u16)) -> Option<LonLat> {
        let (cx, cy) = cursor;
        if cx < self.map_area.x || cy < self.map_area.y {
            return None;
        }
        let local_col = cx - self.map_area.x;
        let local_row = cy - self.map_area.y;
        if local_col >= self.map_area.width || local_row >= self.map_area.height {
            return None;
        }
        self.proj.cell_to_ll(local_col, local_row)
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
    use crate::map::render::overlay::UserPolyline;
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
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
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
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
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

    fn text_at_row(buf: &Buffer, area: Rect, row: u16) -> String {
        (0..area.width)
            .map(|x| buf[(area.x + x, area.y + row)].symbol())
            .collect()
    }

    #[test]
    fn text_anchored_top_right_writes_at_right_edge() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        api.text_anchored(Anchor::TopRight, 0, "ABC", Color::Reset);
        let line = text_at_row(&buf, area, 0);
        assert!(
            line.ends_with("ABC"),
            "TopRight should land at right edge, got {line:?}"
        );
        assert!(
            !line.starts_with("ABC"),
            "TopRight should not be left-aligned, got {line:?}"
        );
    }

    #[test]
    fn text_anchored_bottom_left_writes_at_left_edge_last_row() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        api.text_anchored(Anchor::BottomLeft, 0, "XY", Color::Reset);
        let last_row = text_at_row(&buf, area, area.height - 1);
        assert!(
            last_row.starts_with("XY"),
            "BottomLeft, rows_in=0 should hit the very bottom row, got {last_row:?}"
        );
    }

    #[test]
    fn text_anchored_rows_in_offsets_from_corner() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        // Row 2 from the top-left corner should be the third row.
        api.text_anchored(Anchor::TopLeft, 2, "Q", Color::Reset);
        assert_eq!(buf[(area.x, area.y + 2)].symbol(), "Q");
    }

    #[test]
    fn text_anchored_cjk_right_aligns_by_display_width() {
        // Each CJK char is 2 cells wide, so "千代田" occupies 6 cells.
        // Right-aligned in a 20-column area, the first char should
        // start at column 14, not column 17 (which is what naive
        // chars().count() arithmetic would produce).
        let (mut buf, area, frame, theme) = fixture(20, 1);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        api.text_anchored(Anchor::TopRight, 0, "千代田", Color::Reset);
        assert_eq!(buf[(14, 0)].symbol(), "千");
        assert_eq!(buf[(16, 0)].symbol(), "代");
        assert_eq!(buf[(18, 0)].symbol(), "田");
    }

    #[test]
    fn text_anchored_too_deep_is_noop() {
        let (mut buf, area, frame, theme) = fixture(20, 3);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        // rows_in == height should be rejected (no row to land on).
        api.text_anchored(Anchor::TopLeft, 3, "Q", Color::Reset);
        for x in 0..area.width {
            for y in 0..area.height {
                assert_eq!(buf[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn push_polyline_overlay_appends_to_sink() {
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        {
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
            api.push_polyline_overlay(
                vec![LonLat { lon: 0.0, lat: 0.0 }, LonLat { lon: 1.0, lat: 1.0 }],
                42,
            );
        }
        assert_eq!(sink.len(), 1);
        assert_eq!(sink[0].coords.len(), 2);
        assert_eq!(sink[0].color, 42);
    }
}
