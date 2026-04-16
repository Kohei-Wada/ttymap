//! Info widget — displays coordinates, place name, and scale bar.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph};

use crate::ui::theme;

pub struct InfoWidget {
    coords: String,
    place: Option<String>,
    scale_label: String,
    scale_width: u16,
}

impl Default for InfoWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl InfoWidget {
    pub fn new() -> Self {
        Self {
            coords: String::new(),
            place: None,
            scale_label: String::new(),
            scale_width: 0,
        }
    }

    pub fn set_coords(&mut self, coords: String) {
        self.coords = coords;
    }
    pub fn set_place(&mut self, place: Option<String>) {
        self.place = place;
    }
    pub fn set_scale(&mut self, label: String, width: u16) {
        self.scale_label = label;
        self.scale_width = width;
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect) {
        if map_inner.width < 4 || map_inner.height < 1 {
            return;
        }
        self.render_top_right(f, map_inner);
        self.render_scale_bar(f, map_inner);
    }

    fn render_top_right(&self, f: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        if !self.coords.is_empty() {
            lines.push(self.coords.clone());
        }
        if let Some(ref place) = self.place {
            lines.push(place.clone());
        }
        if lines.is_empty() {
            return;
        }

        let max_width = lines
            .iter()
            .map(|l| display_width(l) as u16 + 2)
            .max()
            .unwrap_or(0);
        let width = max_width.min(area.width);
        let height = (lines.len() as u16).min(area.height);

        let overlay = Rect::new(area.right().saturating_sub(width), area.y, width, height);
        f.render_widget(Clear, overlay);
        let widget = Paragraph::new(lines.join("\n"))
            .style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .alignment(Alignment::Right);
        f.render_widget(widget, overlay);
    }

    fn render_scale_bar(&self, f: &mut Frame, area: Rect) {
        if self.scale_width == 0 || area.height < 2 {
            return;
        }

        let bar = format!(
            "├{}┤ {}",
            "─".repeat((self.scale_width as usize).saturating_sub(2)),
            self.scale_label,
        );

        let width = (display_width(&bar) as u16 + 1).min(area.width);
        let overlay = Rect::new(
            area.right().saturating_sub(width),
            area.bottom().saturating_sub(1),
            width,
            1,
        );
        f.render_widget(Clear, overlay);
        let widget = Paragraph::new(bar)
            .style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .alignment(Alignment::Right);
        f.render_widget(widget, overlay);
    }
}

fn display_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}
