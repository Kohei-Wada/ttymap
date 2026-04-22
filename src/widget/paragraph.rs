//! Paragraph descriptor — the most common widget. Replaces both
//! direct `Paragraph` construction and the old `panel_block` flow
//! (use `framed_title = Some(...)` to draw a bordered container).

use ratatui::widgets::Paragraph as RParagraph;

use crate::theme::UiTheme;

use super::style::TextStyle;
use super::text::{Align, Line};

/// Multiline text descriptor.
///
/// - `framed_title`: when `Some`, host wraps the paragraph in a
///   theme-styled bordered block with this title (`title_align`
///   controls title alignment). Eliminates the need for plugins to
///   know about `Block`.
/// - `scroll_y`: first row to render (for scrollable content like
///   wiki article body).
/// - `align`: horizontal alignment of the paragraph's lines (per-line
///   alignment on `Line.align` takes precedence when set).
#[derive(Clone, Debug, Default)]
pub struct Paragraph {
    pub lines: Vec<Line>,
    pub style: TextStyle,
    pub align: Align,
    pub scroll_y: u16,
    pub framed_title: Option<String>,
    pub title_align: Align,
}

impl Paragraph {
    /// Build the concrete `ratatui::widgets::Paragraph`, applying
    /// the optional bordered block from the active theme. Consumed
    /// (`self`) because the ratatui widget owns its data.
    pub(crate) fn into_ratatui(self, theme: &UiTheme) -> RParagraph<'static> {
        use ratatui::text::Line as RLine;

        let lines: Vec<RLine<'static>> = self.lines.into_iter().map(Into::into).collect();

        let mut p = RParagraph::new(lines)
            .style(self.style)
            .alignment(self.align.into())
            .scroll((self.scroll_y, 0));

        if let Some(title) = self.framed_title {
            let block = theme.panel(&title).title_alignment(self.title_align.into());
            p = p.block(block);
        }

        p
    }
}

#[cfg(test)]
mod tests {
    use super::super::style::{Modifier, TextStyle};
    use super::super::text::Span;
    use super::*;

    #[test]
    fn paragraph_builds_without_title() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let p = Paragraph {
            lines: vec![Line::from_span(Span::raw("hello"))],
            style: TextStyle {
                fg: Some(1),
                bg: None,
                modifier: Modifier::NONE,
            },
            align: Align::Left,
            scroll_y: 0,
            framed_title: None,
            title_align: Align::Left,
        };
        // Just checks it constructs without panic; visual parity
        // covered by smoke tests after C3.
        let _ = p.into_ratatui(&theme);
    }

    #[test]
    fn paragraph_builds_with_title() {
        let theme = UiTheme::from_palette(&crate::theme::DARK);
        let p = Paragraph {
            lines: vec![Line::from_span(Span::raw("x"))],
            style: TextStyle::default(),
            align: Align::Left,
            scroll_y: 3,
            framed_title: Some("panel".into()),
            title_align: Align::Center,
        };
        let _ = p.into_ratatui(&theme);
    }
}
