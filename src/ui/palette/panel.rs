//! Command-palette popup — centered over the map. Single bordered
//! block enclosing an input line (provider prompt + query) and a
//! scrollable `Table` driven by the current provider's items.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState};

use crate::theme::UiTheme;

use super::CommandPalette;

pub fn render_panel(widget: &CommandPalette, f: &mut Frame, map_inner: Rect, theme: &UiTheme) {
    let state = widget.state();
    if !state.active || map_inner.width < 30 || map_inner.height < 6 {
        return;
    }
    let Some(provider) = state.provider.as_ref() else {
        return;
    };
    let items = provider.items();

    let popup_width = (map_inner.width * 2 / 3).max(40).min(map_inner.width - 2);
    // outer borders + input line + blank + table rows
    let max_rows = map_inner.height.saturating_sub(6).max(3);
    let rows = (items.len() as u16).max(1).min(max_rows);
    let popup_height = rows + 4;

    let x = map_inner.x + (map_inner.width - popup_width) / 2;
    let y = map_inner.y + 1;
    let area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, area);

    let block = theme.panel("command palette");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // prompt + query
            Constraint::Length(1), // blank
            Constraint::Min(1),    // table
        ])
        .split(inner);

    let input = Paragraph::new(format!("{}{}", provider.prompt(), state.query)).style(theme.text());
    f.render_widget(input, chunks[0]);

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
                Cell::from(hint_cell).style(theme.muted()),
            ])
        })
        .collect();

    let mut ts = TableState::default();
    if !items.is_empty() {
        ts.select(Some(state.selected));
    }

    let table = Table::new(table_rows, [Constraint::Min(10), Constraint::Length(16)])
        .style(theme.text())
        .highlight_symbol("> ")
        .row_highlight_style(theme.selected())
        .column_spacing(1);

    f.render_stateful_widget(table, chunks[2], &mut ts);
}
