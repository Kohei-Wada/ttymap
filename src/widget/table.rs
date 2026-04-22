//! Table descriptor + helpers.

use ratatui::widgets::{Cell as RCell, Row as RRow, Table as RTable, TableState as RTableState};

use super::geom::Size;
use super::style::TextStyle;

#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub style: TextStyle,
}

impl Cell {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

impl From<Cell> for RCell<'static> {
    fn from(c: Cell) -> Self {
        RCell::from(c.text).style(c.style)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn new(cells: Vec<Cell>) -> Self {
        Self { cells }
    }
}

impl From<Row> for RRow<'static> {
    fn from(r: Row) -> Self {
        let cells: Vec<RCell<'static>> = r.cells.into_iter().map(Into::into).collect();
        RRow::new(cells)
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    pub rows: Vec<Row>,
    pub widths: Vec<Size>,
    pub style: TextStyle,
    pub highlight_symbol: String,
    pub row_highlight_style: TextStyle,
    pub column_spacing: u16,
}

impl From<Table> for RTable<'static> {
    fn from(t: Table) -> Self {
        let rows: Vec<RRow<'static>> = t.rows.into_iter().map(Into::into).collect();
        let widths: Vec<ratatui::layout::Constraint> =
            t.widths.into_iter().map(Into::into).collect();
        RTable::new(rows, widths)
            .style(t.style)
            .highlight_symbol(t.highlight_symbol)
            .row_highlight_style(t.row_highlight_style)
            .column_spacing(t.column_spacing)
    }
}

/// Table selection state. Replaces `ratatui::widgets::TableState`;
/// we drop `offset` because the palette is the only consumer and
/// rows always fit the popup. Add `pub offset: usize` if a future
/// user needs scroll.
#[derive(Clone, Copy, Debug, Default)]
pub struct TableSel {
    pub selected: Option<usize>,
}

impl TableSel {
    pub fn new(selected: Option<usize>) -> Self {
        Self { selected }
    }
}

impl From<TableSel> for RTableState {
    fn from(s: TableSel) -> Self {
        let mut state = RTableState::default();
        state.select(s.selected);
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_builds() {
        let t = Table {
            rows: vec![Row::new(vec![
                Cell::new("left", TextStyle::default()),
                Cell::new("right", TextStyle::default()),
            ])],
            widths: vec![Size::Min(5), Size::Fixed(10)],
            style: TextStyle::default(),
            highlight_symbol: "> ".into(),
            row_highlight_style: TextStyle::default(),
            column_spacing: 1,
        };
        let _: RTable = t.into();
    }

    #[test]
    fn table_sel_select_propagates() {
        let sel = TableSel::new(Some(3));
        let state: RTableState = sel.into();
        assert_eq!(state.selected(), Some(3));
    }
}
