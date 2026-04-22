//! Search side panel — center popup showing the input line or the
//! candidate list. Stateless; reads the component's fields and the
//! theme through the supplied [`RenderWindow`].

use ratatui::layout::Rect;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::compositor::window::RenderWindow;

use super::SearchComponent;

pub fn render_panel(widget: &SearchComponent, win: &mut RenderWindow) {
    let map_inner = win.area();
    if map_inner.width < 10 || map_inner.height < 3 {
        return;
    }

    let popup_width = (map_inner.width * 2 / 3).max(30).min(map_inner.width - 2);
    let popup_height = if widget.has_candidates() {
        (widget.candidates.len() as u16 + 4).min(map_inner.height - 2)
    } else {
        3
    };

    let x = map_inner.x + (map_inner.width - popup_width) / 2;
    let y = map_inner.y + 1;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let title = if widget.has_candidates() {
        format!("search: {}", widget.query)
    } else {
        "search".to_string()
    };
    let inner = win.panel(popup_area, &title);

    if widget.has_candidates() {
        render_candidates(widget, win, inner);
    } else {
        render_input(widget, win, inner);
    }
}

fn render_input(widget: &SearchComponent, win: &mut RenderWindow, area: Rect) {
    let text = Paragraph::new(format!("/{}", widget.query)).style(win.body_style());
    win.render_widget(text, area);
}

fn render_candidates(widget: &SearchComponent, win: &mut RenderWindow, area: Rect) {
    let body = win.body_style();
    let selected = win.selected_style();
    let items: Vec<ListItem> = widget
        .candidates
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let style = if i == widget.selected { selected } else { body };
            let prefix = if i == widget.selected { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, result.name)).style(style)
        })
        .collect();

    let list = List::new(items);
    win.render_widget(list, area);
}
