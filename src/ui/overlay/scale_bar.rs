//! Bottom-right overlay — distance scale bar.
//!
//! Stateless: derives its label and width directly from the `MapFrame`
//! the map was rendered at (via `geo::scale_bar`).

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::geo;
use crate::map::render::frame::MapFrame;
use crate::theme::UiTheme;

use super::MapOverlay;

pub struct ScaleBarOverlay;

impl MapOverlay for ScaleBarOverlay {
    fn render(&self, buf: &mut Buffer, map_area: Rect, frame: &MapFrame, theme: &UiTheme) {
        if map_area.height < 2 {
            return;
        }

        let (label, cells) = geo::scale_bar(frame.center.lat, frame.zoom, map_area.width);
        if cells == 0 {
            return;
        }

        let bar = format!(
            "├{}┤ {}",
            "─".repeat((cells as usize).saturating_sub(2)),
            label
        );

        let width = (bar.width() as u16 + 1).min(map_area.width);
        let rect = Rect::new(
            map_area.right().saturating_sub(width),
            map_area.bottom().saturating_sub(1),
            width,
            1,
        );
        Clear.render(rect, buf);
        Paragraph::new(bar)
            .style(Style::default().fg(theme.accent).bg(theme.bg))
            .alignment(Alignment::Right)
            .render(rect, buf);
    }
}
