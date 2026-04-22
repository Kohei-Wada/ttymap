//! Command-palette popup — centered over the map. Single bordered
//! block enclosing an input line (provider prompt + query) and a
//! scrollable table driven by the current provider's items.

use crate::compositor::window::RenderWindow;
use crate::widget::{
    Cell, Line, Paragraph, Rect, Row, Size, Span, StyleKind, Table, TableSel, split_rows,
};

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

    let chunks = split_rows(
        inner,
        &[
            Size::Fixed(1), // prompt + query
            Size::Fixed(1), // blank
            Size::Min(1),   // table
        ],
    );

    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);
    let selected = win.style(StyleKind::Selected);

    let input_text = Paragraph {
        lines: vec![Line::from_span(Span::styled(
            format!("{}{}", provider.prompt(), widget.query),
            body,
        ))],
        style: body,
        ..Default::default()
    };
    win.paragraph(input_text, chunks[0]);

    let table_rows: Vec<Row> = items
        .iter()
        .map(|item| {
            let hint_cell = if item.hint.is_empty() {
                String::new()
            } else {
                format!("[{}]", item.hint)
            };
            Row::new(vec![
                Cell::new(item.label.clone(), body),
                Cell::new(hint_cell, muted),
            ])
        })
        .collect();

    let sel = TableSel::new(if items.is_empty() {
        None
    } else {
        Some(widget.selected)
    });

    let table = Table {
        rows: table_rows,
        widths: vec![Size::Min(10), Size::Fixed(16)],
        style: body,
        highlight_symbol: "> ".to_string(),
        row_highlight_style: selected,
        column_spacing: 1,
    };

    win.table(table, chunks[2], &sel);
}
