//! Top-right overlay showing the current center's coords and, if a
//! reverse-geocode response has arrived, the human-readable place name.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::render::frame::MapFrame;
use crate::ui::overlay::MapOverlay;
use crate::ui::theme::Theme;

use super::state::InfoState;

pub struct CoordsOverlay<'a> {
    pub state: &'a InfoState,
}

impl MapOverlay for CoordsOverlay<'_> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, _frame: &MapFrame, theme: &Theme) {
        let state = self.state;
        if map_area.width < 4 || map_area.height < 1 {
            return;
        }

        let mut lines: Vec<String> = Vec::new();
        if !state.coords.is_empty() {
            lines.push(state.coords.clone());
        }
        if let Some(ref place) = state.place {
            lines.push(place.clone());
        }
        if lines.is_empty() {
            return;
        }

        let max_width = lines
            .iter()
            .map(|l| l.width() as u16 + 2)
            .max()
            .unwrap_or(0);
        let width = max_width.min(map_area.width);
        let height = (lines.len() as u16).min(map_area.height);

        let rect = Rect::new(
            map_area.right().saturating_sub(width),
            map_area.y,
            width,
            height,
        );
        Clear.render(rect, buf);
        Paragraph::new(lines.join("\n"))
            .style(Style::default().fg(theme.accent).bg(theme.bg))
            .alignment(Alignment::Right)
            .render(rect, buf);
    }
}
