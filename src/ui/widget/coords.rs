//! Top-right overlay — current center's lat/lon and zoom.
//!
//! Stateless: derives its string directly from the `MapFrame` the map
//! was rendered at. Add to the overlay slice to enable; remove to hide.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::render::frame::MapFrame;
use crate::ui::overlay::MapOverlay;
use crate::ui::theme::Theme;

pub struct CoordsOverlay;

impl MapOverlay for CoordsOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &Theme) {
        if map_area.width < 4 || map_area.height < 1 {
            return;
        }

        let line = format!(
            " {:.3}, {:.3}  zoom: {:.1}",
            frame.center.lat, frame.center.lon, frame.zoom
        );

        let width = (line.width() as u16 + 2).min(map_area.width);
        let rect = Rect::new(map_area.right().saturating_sub(width), map_area.y, width, 1);
        Clear.render(rect, buf);
        Paragraph::new(line)
            .style(Style::default().fg(theme.accent).bg(theme.bg))
            .alignment(Alignment::Right)
            .render(rect, buf);
    }
}
