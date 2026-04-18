//! Top-right overlay — human-readable place name for the current
//! center, resolved asynchronously via reverse geocoding.
//!
//! Unlike `coords` / `scale_bar`, place is not derivable from the
//! `MapFrame`: it comes from an external Nominatim response arriving on
//! the app event loop. `app.rs` pushes the result via `set_name`; the
//! overlay reads it.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;

use super::MapOverlay;

pub struct PlaceState {
    name: Option<String>,
}

impl Default for PlaceState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaceState {
    pub fn new() -> Self {
        Self { name: None }
    }

    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }
}

pub struct PlaceOverlay<'a> {
    pub state: &'a PlaceState,
}

impl MapOverlay for PlaceOverlay<'_> {
    fn render(&self, buf: &mut Buffer, map_area: Rect, _frame: &MapFrame, theme: &Theme) {
        let Some(ref name) = self.state.name else {
            return;
        };
        if map_area.width < 4 || map_area.height < 2 {
            return;
        }

        // Row 1 (just below the coords overlay).
        let width = (name.width() as u16 + 2).min(map_area.width);
        let rect = Rect::new(
            map_area.right().saturating_sub(width),
            map_area.y + 1,
            width,
            1,
        );
        Clear.render(rect, buf);
        Paragraph::new(name.clone())
            .style(Style::default().fg(theme.accent).bg(theme.bg))
            .alignment(Alignment::Right)
            .render(rect, buf);
    }
}
