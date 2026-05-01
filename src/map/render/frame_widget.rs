//! ratatui `Widget` adapter for [`MapFrame`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;

use super::frame::MapFrame;

impl Widget for &MapFrame {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let draw_cols = self.cols.min(area.width);
        let draw_rows = self.rows.min(area.height);

        for row in 0..draw_rows {
            for col in 0..draw_cols {
                let cell = &self.cells[(row * self.cols + col) as usize];
                let x = area.x + col;
                let y = area.y + row;
                if x < area.x + area.width && y < area.y + area.height {
                    let target = &mut buf[(x, y)];
                    target
                        .set_char(cell.ch)
                        .set_fg(xterm_to_color(cell.fg))
                        .set_bg(xterm_to_color(cell.bg));
                    if cell.dim {
                        target.modifier.insert(Modifier::DIM);
                    }
                }
            }
        }
    }
}

/// Convert xterm-256 color index to ratatui Color.
fn xterm_to_color(idx: u8) -> Color {
    if idx == 0 {
        Color::Reset
    } else {
        Color::Indexed(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::render::frame::MapCell;

    #[test]
    fn test_xterm_to_color() {
        assert_eq!(xterm_to_color(0), Color::Reset);
        assert_eq!(xterm_to_color(1), Color::Indexed(1));
        assert_eq!(xterm_to_color(255), Color::Indexed(255));
    }

    #[test]
    fn test_map_frame_widget_render() {
        let frame = MapFrame {
            cells: vec![
                MapCell {
                    ch: 'A',
                    fg: 1,
                    bg: 0,
                    dim: false,
                },
                MapCell {
                    ch: 'B',
                    fg: 2,
                    bg: 0,
                    dim: false,
                },
            ],
            cols: 2,
            rows: 1,
            center: crate::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        };
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        (&frame).render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(1, 0)].symbol(), "B");
    }

    #[test]
    fn dim_flag_applies_modifier_dim() {
        use ratatui::style::Modifier;

        let frame = MapFrame {
            cells: vec![
                MapCell {
                    ch: 'A',
                    fg: 1,
                    bg: 0,
                    dim: false,
                },
                MapCell {
                    ch: 'B',
                    fg: 2,
                    bg: 0,
                    dim: true,
                },
            ],
            cols: 2,
            rows: 1,
            center: crate::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        };
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        (&frame).render(area, &mut buf);
        assert!(
            !buf[(0, 0)].modifier.contains(Modifier::DIM),
            "dim=false should not add DIM"
        );
        assert!(
            buf[(1, 0)].modifier.contains(Modifier::DIM),
            "dim=true should add DIM"
        );
    }
}
