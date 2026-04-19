//! Bottom-left overlay — static tile attribution string.
//! Skips rendering if text is empty or map area too narrow.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::map::render::frame::MapFrame;
use crate::theme::UiTheme;

use super::MapOverlay;

pub struct AttributionOverlay<'a> {
    pub text: &'a str,
}

impl<'a> MapOverlay for AttributionOverlay<'a> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, _frame: &MapFrame, theme: &UiTheme) {
        if self.text.is_empty() || map_area.height < 2 {
            return;
        }
        let w = self.text.width() as u16;
        if w == 0 || w > map_area.width {
            return;
        }
        let rect = Rect::new(map_area.left(), map_area.bottom().saturating_sub(1), w, 1);
        Clear.render(rect, buf);
        Paragraph::new(self.text)
            .style(Style::default().fg(theme.muted_color).bg(theme.bg))
            .alignment(Alignment::Left)
            .render(rect, buf);
    }
}
