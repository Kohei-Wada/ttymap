//! Command-palette popup — centered over the map. Single bordered
//! block enclosing an input line and a scrollable `Table` of commands.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState};

use crate::ui::theme::Theme;

use super::PalettePlugin;

pub fn render_panel(widget: &PalettePlugin, f: &mut Frame, map_inner: Rect, theme: &Theme) {
    let state = &widget.state;
    if !state.active || map_inner.width < 30 || map_inner.height < 6 {
        return;
    }

    let popup_width = (map_inner.width * 2 / 3).max(40).min(map_inner.width - 2);
    // outer borders + input line + blank + table rows
    let max_rows = map_inner.height.saturating_sub(6).max(3);
    let rows = (state.filtered.len() as u16).max(1).min(max_rows);
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
            Constraint::Length(1), // ":query"
            Constraint::Length(1), // blank
            Constraint::Min(1),    // table
        ])
        .split(inner);

    let input = Paragraph::new(format!(":{}", state.query)).style(theme.text());
    f.render_widget(input, chunks[0]);

    render_table(widget, f, chunks[2], theme);
}

fn render_table(widget: &PalettePlugin, f: &mut Frame, area: Rect, theme: &Theme) {
    let state = &widget.state;

    let rows: Vec<Row> = state
        .filtered
        .iter()
        .map(|&i| {
            let cmd = &state.commands[i];
            let keys = if cmd.keys.is_empty() {
                String::new()
            } else {
                format!("[{}]", cmd.keys)
            };
            Row::new(vec![
                Cell::from(cmd.label.clone()),
                Cell::from(keys).style(theme.muted()),
            ])
        })
        .collect();

    let mut ts = TableState::default();
    if !state.filtered.is_empty() {
        ts.select(Some(state.selected));
    }

    let table = Table::new(rows, [Constraint::Min(10), Constraint::Length(16)])
        .highlight_symbol("> ")
        .row_highlight_style(theme.selected())
        .column_spacing(1);

    f.render_stateful_widget(table, area, &mut ts);
}
