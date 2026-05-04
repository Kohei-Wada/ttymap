//! ratatui `Widget` adapter for [`MapFrame`].
//!
//! `MapFrame` is defined in `ttymap-engine` (which has no ratatui
//! dependency), so we can't `impl Widget for &MapFrame` directly —
//! that violates the orphan rule. The wrapper [`MapFrameWidget`] is
//! a binary-side newtype that satisfies coherence: callers use it
//! as `f.render_widget(MapFrameWidget(&frame), area)`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use ttymap_engine::map::render::frame::MapFrame;

pub struct MapFrameWidget<'a>(pub &'a MapFrame);

impl<'a> Widget for MapFrameWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let frame = self.0;
        let draw_cols = frame.cols.min(area.width);
        let draw_rows = frame.rows.min(area.height);

        for row in 0..draw_rows {
            for col in 0..draw_cols {
                let cell = &frame.cells[(row * frame.cols + col) as usize];
                let x = area.x + col;
                let y = area.y + row;
                if x < area.x + area.width && y < area.y + area.height {
                    let target = &mut buf[(x, y)];
                    target
                        .set_char(cell.ch)
                        .set_fg(xterm_to_color(cell.fg))
                        .set_bg(xterm_to_color(cell.bg));
                }
            }
        }
    }
}

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
    use ttymap_engine::map::render::frame::MapCell;

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
                },
                MapCell {
                    ch: 'B',
                    fg: 2,
                    bg: 0,
                },
            ],
            cols: 2,
            rows: 1,
            center: ttymap_engine::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        };
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        MapFrameWidget(&frame).render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(1, 0)].symbol(), "B");
    }
}
