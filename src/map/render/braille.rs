use super::frame::{MapCell, MapFrame};

/// Braille pixel buffer — maps a 2D pixel grid onto Unicode Braille characters (U+2800..U+28FF).
/// Each terminal character cell = 2 pixels wide × 4 pixels tall.
///
/// Bit layout per cell:
/// ```text
/// [0x01] [0x08]
/// [0x02] [0x10]
/// [0x04] [0x20]
/// [0x40] [0x80]
/// ```
const BRAILLE_MAP: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

pub struct BrailleBuffer {
    width: usize,
    height: usize,
    /// One byte per cell; bits indicate which braille dots are set.
    pixel_buf: Vec<u8>,
    /// Foreground color index (256-color) per cell.
    fg_buf: Vec<u8>,
    /// Background color index (256-color) per cell.
    bg_buf: Vec<u8>,
    /// Optional character overlay per cell; takes priority over braille rendering.
    char_buf: Vec<Option<char>>,
    /// Color for char_buf entries.
    char_color_buf: Vec<u8>,
    /// Optional global background color applied when no per-cell bg is set.
    global_bg: Option<u8>,
}

impl BrailleBuffer {
    /// Create a new buffer. `width` and `height` are in pixels.
    /// Cell count = (width / 2) * (height / 4).
    pub fn new(width: usize, height: usize) -> Self {
        let cell_count = (width / 2) * (height / 4);
        Self {
            width,
            height,
            pixel_buf: vec![0u8; cell_count],
            fg_buf: vec![0u8; cell_count],
            bg_buf: vec![0u8; cell_count],
            char_buf: vec![None; cell_count],
            char_color_buf: vec![0u8; cell_count],
            global_bg: None,
        }
    }

    /// Project pixel (x, y) to cell index.
    #[inline]
    fn cell_index(&self, x: usize, y: usize) -> usize {
        (x / 2) + (self.width / 2) * (y / 4)
    }

    /// Zero all buffers.
    pub fn clear(&mut self) {
        self.pixel_buf.fill(0);
        self.fg_buf.fill(0);
        self.bg_buf.fill(0);
        self.char_buf.fill(None);
        self.char_color_buf.fill(0);
    }

    /// Set the global background color index.
    pub fn set_global_background(&mut self, color: u8) {
        self.global_bg = Some(color);
    }

    /// Set the braille pixel at pixel coordinates (x, y) with a foreground color.
    /// Out-of-bounds coordinates are silently ignored.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = self.cell_index(x, y);
        let bit = BRAILLE_MAP[y & 3][x & 1];
        self.pixel_buf[idx] |= bit;
        self.fg_buf[idx] = color;
    }

    /// Write a single character at pixel position (x, y), overriding any braille content.
    /// Out-of-bounds coordinates are silently ignored.
    pub fn set_char(&mut self, ch: char, x: usize, y: usize, color: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = self.cell_index(x, y);
        self.char_buf[idx] = Some(ch);
        self.char_color_buf[idx] = color;
    }

    /// Write a string starting at pixel position (x, y).
    /// Advances by character display width (2 pixels per terminal column).
    pub fn write_text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        let mut offset = 0usize;
        for ch in text.chars() {
            self.set_char(ch, x + offset * 2, y, color);
            offset += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        }
    }

    /// Render the buffer to a structured MapFrame for ratatui rendering.
    pub fn to_map_frame(&self) -> MapFrame {
        let cols = self.width / 2;
        let rows = self.height / 4;
        let mut cells = Vec::with_capacity(cols * rows);

        for row in 0..rows {
            let mut skip = 0u32;

            for col in 0..cols {
                let idx = col + cols * row;

                let (fg, bg) = if self.char_buf[idx].is_some() {
                    (self.char_color_buf[idx], self.bg_buf[idx])
                } else {
                    (self.fg_buf[idx], self.bg_buf[idx])
                };

                let effective_bg = if bg != 0 {
                    bg
                } else {
                    self.global_bg.unwrap_or(0)
                };

                if let Some(ch) = self.char_buf[idx] {
                    if skip > 0 {
                        skip -= 1;
                        cells.push(MapCell {
                            ch: ' ',
                            fg: 0,
                            bg: effective_bg,
                        });
                    } else {
                        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                        if w > 1 {
                            skip = (w - 1) as u32;
                        }
                        cells.push(MapCell {
                            ch,
                            fg,
                            bg: effective_bg,
                        });
                    }
                } else if skip > 0 {
                    skip -= 1;
                    cells.push(MapCell {
                        ch: ' ',
                        fg: 0,
                        bg: effective_bg,
                    });
                } else {
                    let codepoint = 0x2800u32 + self.pixel_buf[idx] as u32;
                    let ch = char::from_u32(codepoint).unwrap_or('?');
                    cells.push(MapCell {
                        ch,
                        fg,
                        bg: effective_bg,
                    });
                }
            }
        }

        MapFrame {
            cells,
            cols: cols as u16,
            rows: rows as u16,
            center: crate::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer_dimensions() {
        let buf = BrailleBuffer::new(80, 24);
        assert_eq!(buf.width, 80);
        assert_eq!(buf.height, 24);
        let expected_cells = (80 / 2) * (24 / 4);
        assert_eq!(buf.pixel_buf.len(), expected_cells);
        assert_eq!(buf.fg_buf.len(), expected_cells);
        assert_eq!(buf.bg_buf.len(), expected_cells);
        assert_eq!(buf.char_buf.len(), expected_cells);
        assert_eq!(buf.char_color_buf.len(), expected_cells);
    }

    #[test]
    fn test_set_pixel_and_frame_not_empty() {
        let mut buf = BrailleBuffer::new(4, 8);
        buf.set_pixel(0, 0, 7);
        let frame = buf.to_map_frame();
        // U+2801 is the braille char with dot 1 set (bit 0x01)
        assert!(
            frame.cells.iter().any(|c| c.ch == '\u{2801}'),
            "frame should contain U+2801 (dot 1 set)"
        );
    }

    #[test]
    fn test_set_pixel_out_of_bounds_no_panic() {
        let mut buf = BrailleBuffer::new(4, 8);
        // These should not panic
        buf.set_pixel(100, 100, 1);
        buf.set_pixel(4, 0, 1);
        buf.set_pixel(0, 8, 1);
        buf.set_pixel(usize::MAX, usize::MAX, 1);
    }

    #[test]
    fn test_clear_resets_pixels() {
        let mut buf = BrailleBuffer::new(4, 8);
        buf.set_pixel(0, 0, 7);
        buf.set_pixel(1, 1, 3);
        buf.clear();
        for &b in &buf.pixel_buf {
            assert_eq!(b, 0, "pixel_buf should be zeroed after clear");
        }
        for &b in &buf.fg_buf {
            assert_eq!(b, 0, "fg_buf should be zeroed after clear");
        }
        for c in &buf.char_buf {
            assert!(c.is_none(), "char_buf should be None after clear");
        }
    }

    #[test]
    fn test_write_text_sets_chars() {
        let mut buf = BrailleBuffer::new(8, 8);
        buf.write_text("AB", 0, 0, 15);
        let frame = buf.to_map_frame();
        assert!(
            frame.cells.iter().any(|c| c.ch == 'A'),
            "frame should contain 'A'"
        );
        assert!(
            frame.cells.iter().any(|c| c.ch == 'B'),
            "frame should contain 'B'"
        );
    }

    #[test]
    fn test_frame_dimensions() {
        // 8x8 pixel buffer → 4 cols × 2 rows of cells
        let buf = BrailleBuffer::new(8, 8);
        let frame = buf.to_map_frame();
        assert_eq!(frame.cols, 4);
        assert_eq!(frame.rows, 2);
        assert_eq!(frame.cells.len(), 8);
    }

    #[test]
    fn test_braille_map_bit_layout() {
        // Verify bit assignments match the spec
        assert_eq!(BRAILLE_MAP[0][0], 0x01);
        assert_eq!(BRAILLE_MAP[0][1], 0x08);
        assert_eq!(BRAILLE_MAP[1][0], 0x02);
        assert_eq!(BRAILLE_MAP[1][1], 0x10);
        assert_eq!(BRAILLE_MAP[2][0], 0x04);
        assert_eq!(BRAILLE_MAP[2][1], 0x20);
        assert_eq!(BRAILLE_MAP[3][0], 0x40);
        assert_eq!(BRAILLE_MAP[3][1], 0x80);
    }

    #[test]
    fn test_set_global_background() {
        let mut buf = BrailleBuffer::new(4, 4);
        buf.set_global_background(9);
        assert_eq!(buf.global_bg, Some(9));
    }
}
