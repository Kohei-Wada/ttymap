//! Bottom-right overlay showing a distance scale bar for the current
//! zoom / latitude.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::render::frame::MapFrame;
use crate::ui::overlay::MapOverlay;
use crate::ui::theme::Theme;

use super::state::InfoState;

pub struct ScaleBarOverlay<'a> {
    pub state: &'a InfoState,
}

impl MapOverlay for ScaleBarOverlay<'_> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, _frame: &MapFrame, theme: &Theme) {
        let state = self.state;
        if state.scale_width == 0 || map_area.height < 2 {
            return;
        }

        let bar = format!(
            "├{}┤ {}",
            "─".repeat((state.scale_width as usize).saturating_sub(2)),
            state.scale_label,
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
