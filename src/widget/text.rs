//! Text primitives — `Align`, `Span`, `Line`. Mirrors of
//! `ratatui::layout::Alignment`, `ratatui::text::Span`,
//! `ratatui::text::Line`.

use std::borrow::Cow;

use ratatui::layout::Alignment;
use ratatui::text::{Line as RLine, Span as RSpan};

use super::style::TextStyle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl From<Align> for Alignment {
    fn from(a: Align) -> Self {
        match a {
            Align::Left => Alignment::Left,
            Align::Center => Alignment::Center,
            Align::Right => Alignment::Right,
        }
    }
}

/// Styled text fragment. Owns its text as `Cow<'static, str>` —
/// accept either `&'static str` literals (zero-copy) or owned
/// `String`. Borrowing from a non-static local is not supported;
/// callers clone or own.
#[derive(Clone, Debug, Default)]
pub struct Span {
    pub text: Cow<'static, str>,
    pub style: TextStyle,
}

impl Span {
    pub fn raw(text: impl Into<Cow<'static, str>>) -> Self {
        Self {
            text: text.into(),
            style: TextStyle::default(),
        }
    }

    pub fn styled(text: impl Into<Cow<'static, str>>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

impl From<Span> for RSpan<'static> {
    fn from(s: Span) -> Self {
        RSpan::styled(s.text, s.style)
    }
}

/// Ordered collection of spans rendered as a single line with an
/// alignment hint.
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub spans: Vec<Span>,
    pub align: Align,
}

impl Line {
    pub fn from_span(s: Span) -> Self {
        Self {
            spans: vec![s],
            align: Align::default(),
        }
    }

    pub fn from_spans(spans: Vec<Span>) -> Self {
        Self {
            spans,
            align: Align::default(),
        }
    }

    pub fn aligned(spans: Vec<Span>, align: Align) -> Self {
        Self { spans, align }
    }
}

impl From<Line> for RLine<'static> {
    fn from(l: Line) -> Self {
        let spans: Vec<RSpan<'static>> = l.spans.into_iter().map(Into::into).collect();
        RLine::from(spans).alignment(l.align.into())
    }
}

#[cfg(test)]
mod tests {
    use super::super::style::{Modifier, TextStyle};
    use super::*;

    #[test]
    fn align_to_ratatui() {
        assert!(matches!(Alignment::from(Align::Left), Alignment::Left));
        assert!(matches!(Alignment::from(Align::Center), Alignment::Center));
        assert!(matches!(Alignment::from(Align::Right), Alignment::Right));
    }

    #[test]
    fn span_raw_preserves_text() {
        let s = Span::raw("hello");
        let r: RSpan = s.into();
        assert_eq!(r.content, "hello");
    }

    #[test]
    fn span_styled_preserves_style() {
        let s = Span::styled(
            "x",
            TextStyle {
                fg: Some(9),
                bg: None,
                modifier: Modifier::BOLD,
            },
        );
        let r: RSpan = s.into();
        assert_eq!(r.content, "x");
        assert_eq!(r.style.fg, Some(ratatui::style::Color::Indexed(9)));
        assert!(r.style.add_modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn line_from_spans_carries_alignment() {
        let line = Line::aligned(vec![Span::raw("a"), Span::raw("b")], Align::Center);
        let r: RLine = line.into();
        assert_eq!(r.alignment, Some(Alignment::Center));
        assert_eq!(r.spans.len(), 2);
    }
}
