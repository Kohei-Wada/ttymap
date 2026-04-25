//! Help component — center-popup overlay pushed onto the
//! compositor stack. Modal: any key closes it.

use std::rc::Rc;

use crossterm::event::KeyEvent;

use crate::plugin_api::prelude::*;

use super::text::{HelpText, line_width};

pub struct HelpComponent {
    text: Rc<HelpText>,
}

impl HelpComponent {
    pub fn new(text: Rc<HelpText>) -> Self {
        Self { text }
    }
}

impl Component for HelpComponent {
    fn handle_event(&mut self, _event: KeyEvent, win: &mut Window) {
        // Help is fully modal: any key closes the panel. (Tab is
        // intercepted by the compositor before it reaches here.)
        win.close();
    }

    fn render(&self, win: &mut RenderWindow) {
        let map_inner = win.area();
        if map_inner.width < 20 || map_inner.height < 10 {
            return;
        }

        let rendered = self.text.rendered_lines(win);

        let content_width = rendered.iter().map(line_width).max().unwrap_or(30) + 6;
        let content_height = rendered.len() as u16 + 2;

        let max_width = map_inner.width.saturating_sub(4).max(20);
        let max_height = map_inner.height.saturating_sub(2).max(10);
        let popup_width = content_width.clamp(50, max_width);
        let popup_height = content_height.min(max_height);

        let x = map_inner.x + (map_inner.width - popup_width) / 2;
        let y = map_inner.y + (map_inner.height - popup_height) / 2;

        let area = Rect::new(x, y, popup_width, popup_height);
        let body = win.style(StyleKind::Body);
        let paragraph = Paragraph {
            lines: rendered,
            style: body,
            framed_title: Some("help".to_string()),
            title_align: Align::Center,
            ..Default::default()
        };
        win.clear(area);
        win.paragraph(paragraph, area);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("any key", "close")]
    }

    fn name(&self) -> &'static str {
        "help"
    }
}
