//! Reusable list panel — frames + windowing for scrollable lists.
//!
//! Plugins like wiki and aircraft show a title, an optional
//! sub-header, and a scrollable list of rows with one row
//! highlighted. The framing, the "(empty)" fallback, and the
//! window-around-selected math are identical across them, so they
//! live here instead of being copy-pasted into each panel.rs.
//!
//! The plugin still owns row formatting — pre-built [`Line`]s carry
//! whatever per-item layout (columns, accent colours, glyphs) the
//! plugin wants. `ListPanel` is just the surrounding chrome.
//!
//! ```ignore
//! let rows: Vec<Line> = state.aircraft.iter().enumerate().map(|(i, a)| {
//!     let style = if i == state.selected { selected } else { body };
//!     Line::from_span(Span::styled(format!(" {} {}m", a.callsign, a.alt), style))
//! }).collect();
//!
//! ListPanel {
//!     title: "aircraft".into(),
//!     subtitle: Some(format!("{} tracked", state.aircraft.len())),
//!     rows,
//!     selected: state.selected,
//!     empty: "(no data yet)".into(),
//! }
//! .render(area, win);
//! ```

use crate::compositor::window::RenderWindow;
use crate::widget::{Line, Paragraph, Rect, Span, StyleKind};

pub struct ListPanel {
    /// Framed title shown in the top border.
    pub title: String,
    /// Optional sub-header drawn as the first body line in muted
    /// style — typically a count or status (e.g. "5 tracked").
    pub subtitle: Option<String>,
    /// Pre-styled body rows. The plugin formats each row however it
    /// wants; `ListPanel` just windows the slice and renders.
    pub rows: Vec<Line>,
    /// Index of the highlighted row. Used only for windowing — the
    /// caller is responsible for the visual highlight inside the
    /// `Line`'s spans.
    pub selected: usize,
    /// Body text shown when `rows` is empty.
    pub empty: String,
}

impl ListPanel {
    pub fn render(self, area: Rect, win: &mut RenderWindow) {
        let body = win.style(StyleKind::Body);
        let muted = win.style(StyleKind::Muted);
        win.clear(area);

        // Available rows inside the framed border (top + bottom = 2).
        let mut budget = (area.height as usize).saturating_sub(2);
        let mut lines: Vec<Line> = Vec::new();

        if let Some(sub) = self.subtitle {
            lines.push(Line::from_span(Span::styled(format!(" {}", sub), muted)));
            budget = budget.saturating_sub(1);
        }

        if self.rows.is_empty() {
            lines.push(Line::from_span(Span::styled(
                format!(" {}", self.empty),
                muted,
            )));
        } else {
            // Centre the visible window around `selected`, clamped so
            // it never starts past total - visible.
            let total = self.rows.len();
            let visible = budget.min(total);
            if visible > 0 {
                let half = visible / 2;
                let start = self
                    .selected
                    .saturating_sub(half)
                    .min(total.saturating_sub(visible));
                let end = (start + visible).min(total);
                lines.extend(self.rows.into_iter().skip(start).take(end - start));
            }
        }

        let paragraph = Paragraph {
            lines,
            style: body,
            framed_title: Some(self.title),
            ..Default::default()
        };
        win.paragraph(paragraph, area);
    }
}
