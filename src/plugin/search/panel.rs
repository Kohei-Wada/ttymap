//! Search side panel — center popup showing the input line or the
//! candidate list. Stateless; reads the widget's internal widget.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, List, ListItem, Paragraph};

use crate::theme::UiTheme;

use super::SearchPlugin;

pub fn render_panel(widget: &SearchPlugin, f: &mut Frame, map_inner: Rect, theme: &UiTheme) {
    if !widget.active || map_inner.width < 10 || map_inner.height < 3 {
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
    f.render_widget(Clear, popup_area);

    if widget.has_candidates() {
        render_candidates(widget, f, popup_area, theme);
    } else {
        render_input(widget, f, popup_area, theme);
    }
}

fn render_input(widget: &SearchPlugin, f: &mut Frame, area: Rect, theme: &UiTheme) {
    let block = theme.panel("search");
    let w = Paragraph::new(format!("/{}", widget.query))
        .style(theme.text())
        .block(block);
    f.render_widget(w, area);
}

fn render_candidates(widget: &SearchPlugin, f: &mut Frame, area: Rect, theme: &UiTheme) {
    let title = format!("search: {}", widget.query);
    let block = theme.panel(&title);

    let items: Vec<ListItem> = widget
        .candidates
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let style = if i == widget.selected {
                theme.selected()
            } else {
                theme.text()
            };
            let prefix = if i == widget.selected { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, result.name)).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}
