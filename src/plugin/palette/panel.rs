//! Command-palette popup — centered over the map. Single bordered
//! block enclosing an input line (provider prompt + query) and a
//! scrollable [`Table`] driven by the current provider's items.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::compositor::window::RenderWindow;

use super::PaletteComponent;

pub fn render_panel(widget: &PaletteComponent, win: &mut RenderWindow) {
    let map_inner = win.area();
    if map_inner.width < 30 || map_inner.height < 6 {
        return;
    }
    let provider = &widget.provider;
    let items = provider.items();

    let popup_width = (map_inner.width * 2 / 3).max(40).min(map_inner.width - 2);
    let max_rows = map_inner.height.saturating_sub(6).max(3);
    let rows = (items.len() as u16).max(1).min(max_rows);
    let popup_height = rows + 4;

    let x = map_inner.x + (map_inner.width - popup_width) / 2;
    let y = map_inner.y + 1;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let inner = win.panel(popup_area, "command palette");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // prompt + query
            Constraint::Length(1), // blank
            Constraint::Min(1),    // table
        ])
        .split(inner);

    let theme = win.theme();
    let text_style = theme.text();
    let muted_style = theme.muted();
    let selected_style = theme.selected();

    let input_text =
        Paragraph::new(format!("{}{}", provider.prompt(), widget.query)).style(text_style);
    win.frame().render_widget(input_text, chunks[0]);

    let table_rows: Vec<Row> = items
        .iter()
        .map(|item| {
            let hint_cell = if item.hint.is_empty() {
                String::new()
            } else {
                format!("[{}]", item.hint)
            };
            Row::new(vec![
                Cell::from(item.label.clone()),
                Cell::from(hint_cell).style(muted_style),
            ])
        })
        .collect();

    let mut ts = TableState::default();
    if !items.is_empty() {
        ts.select(Some(widget.selected));
    }

    let table = Table::new(table_rows, [Constraint::Min(10), Constraint::Length(16)])
        .style(text_style)
        .highlight_symbol("> ")
        .row_highlight_style(selected_style)
        .column_spacing(1);

    win.frame()
        .render_stateful_widget(table, chunks[2], &mut ts);
}
