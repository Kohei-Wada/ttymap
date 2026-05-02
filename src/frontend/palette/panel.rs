//! Command-palette popup — centered over the map. Single bordered
//! block enclosing an input line (provider prompt + query) and a
//! scrollable table driven by the current provider's items.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::frontend::compositor::window::RenderWindow;
use crate::theme::StyleKind;

use super::PaletteComponent;

/// Hard cap on visible candidate rows. Keeps the popup compact even
/// on large terminals and turns the table into a fixed-height
/// scrollable viewport — ratatui's `Table` auto-scrolls to keep the
/// selected row in view when items overflow this window.
const MAX_VISIBLE_ROWS: u16 = 10;

pub fn render_panel(widget: &PaletteComponent, win: &mut RenderWindow) {
    let map_inner = win.area();
    if map_inner.width < 30 || map_inner.height < 6 {
        return;
    }
    let provider = &widget.provider;
    let items = provider.items();

    let popup_width = (map_inner.width * 2 / 3).max(40).min(map_inner.width - 2);
    let max_rows = map_inner
        .height
        .saturating_sub(6)
        .clamp(3, MAX_VISIBLE_ROWS);
    let loading = widget.is_loading();
    let visible = items.len() as u16 + u16::from(loading && items.is_empty());
    let rows = visible.max(1).min(max_rows);
    let popup_height = rows + 4;

    let x = map_inner.x + (map_inner.width - popup_width) / 2;
    let y = map_inner.y + 1;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Title carries the scroll position so users can tell at a
    // glance which slice of the candidate list they're looking at,
    // especially after the cap turned the table into a viewport.
    let title = if items.is_empty() {
        "command palette".to_string()
    } else {
        format!("command palette · {}/{}", widget.selected + 1, items.len())
    };
    let inner = win.panel(popup_area, &title);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // prompt + query
            Constraint::Length(1), // blank
            Constraint::Min(1),    // table
        ])
        .split(inner);

    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);
    let selected = win.style(StyleKind::Selected);

    let input_text = Paragraph::new(Line::from(Span::styled(
        format!("{}{}", provider.prompt(), widget.query),
        body,
    )))
    .style(body);
    win.paragraph(input_text, chunks[0]);

    let mut table_rows: Vec<Row<'static>> = items
        .iter()
        .map(|item| {
            let hint_cell = if item.hint.is_empty() {
                String::new()
            } else {
                format!("[{}]", item.hint)
            };
            Row::new(vec![
                Cell::from(item.label.clone()).style(body),
                Cell::from(hint_cell).style(muted),
            ])
        })
        .collect();

    // Surface the loading state as a non-selectable row when the
    // provider hasn't returned anything yet, so the user can tell
    // their input registered. Once results arrive the row drops out.
    if loading && items.is_empty() {
        table_rows.push(Row::new(vec![
            Cell::from("…".to_string()).style(muted),
            Cell::from(String::new()).style(muted),
        ]));
    }

    let mut state = TableState::default();
    state.select(if items.is_empty() {
        None
    } else {
        Some(widget.selected)
    });

    let table = Table::new(table_rows, [Constraint::Min(10), Constraint::Length(16)])
        .style(body)
        .highlight_symbol("> ")
        .row_highlight_style(selected)
        .column_spacing(1);

    win.table(table, chunks[2], &mut state);
}
