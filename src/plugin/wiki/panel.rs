//! Wiki side panel — list view and detail view.
//!
//! Stateless renderer; reads `WikiState` and draws through the
//! supplied [`RenderWindow`] — every style comes from `win`'s
//! semantic accessors, so the panel never touches `UiTheme`.

use unicode_width::UnicodeWidthStr;

use crate::compositor::window::RenderWindow;
use crate::widget::{Line, Paragraph, Rect, Span, StyleKind};

use super::WikiState;
use super::wikipedia::WikiArticle;

/// Render the wiki side panel (list or detail view). Caller ensures
/// the panel is supposed to be up (compositor only calls this while
/// `WikiComponent` is on the stack).
pub fn render_panel(widget: &WikiState, win: &mut RenderWindow) {
    let map_inner = win.area();
    if map_inner.width < 30 || map_inner.height < 6 {
        return;
    }

    let panel_width = (map_inner.width / 4).max(25).min(map_inner.width / 3);
    let y = map_inner.y + 3;
    let panel_height = map_inner.height.saturating_sub(6);

    if panel_height < 4 {
        return;
    }

    let x = map_inner.right().saturating_sub(panel_width + 1);
    let area = Rect::new(x, y, panel_width, panel_height);
    win.clear(area);

    let content_width = (panel_width as usize).saturating_sub(4).max(10);

    if let Some(ref article) = widget.detail {
        render_detail(win, area, content_width, article);
    } else {
        render_list(widget, win, area, panel_height, content_width);
    }
}

fn render_list(
    widget: &WikiState,
    win: &mut RenderWindow,
    area: Rect,
    panel_height: u16,
    content_width: usize,
) {
    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);
    let accent = win.style(StyleKind::Accent);
    let highlight = win.style(StyleKind::Highlight);
    let muted_fg = win.style(StyleKind::MutedFg);

    if widget.articles.is_empty() {
        let paragraph = Paragraph {
            lines: vec![Line::from_span(Span::styled("  Loading...", muted))],
            style: muted,
            framed_title: Some("wiki (Enter: open)".to_string()),
            ..Default::default()
        };
        win.paragraph(paragraph, area);
        return;
    }

    let sep_text = "─".repeat(content_width);
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_top: u16 = 0;
    let mut selected_height: u16 = 1;

    for (i, article) in widget.articles.iter().enumerate() {
        let article_start = lines.len() as u16;

        if i > 0 {
            lines.push(Line::from_span(Span::styled(sep_text.clone(), muted_fg)));
        }

        let is_selected = i == widget.selected;
        let dist = crate::geo::format_distance(article.dist_m);
        let title_style = if is_selected { highlight } else { accent };
        lines.push(Line::from_spans(vec![
            Span::styled(article.title.clone(), title_style),
            Span::styled(format!("  {}", dist), muted),
        ]));

        if !article.extract.is_empty() {
            let max_chars = content_width * 2;
            let raw: String = article.extract.chars().take(max_chars).collect();
            let truncated = if article.extract.chars().count() > max_chars {
                format!("{}...", raw)
            } else {
                raw
            };
            for wrapped in wrap_to_width(&truncated, content_width) {
                lines.push(Line::from_span(Span::styled(wrapped, body)));
            }
        }

        if is_selected {
            selected_top = article_start;
            selected_height = (lines.len() as u16).saturating_sub(article_start).max(1);
        }
    }

    let visible_lines = panel_height.saturating_sub(2);
    let total_lines = lines.len() as u16;
    let max_scroll = total_lines.saturating_sub(visible_lines);
    let scroll = if selected_top + selected_height > visible_lines {
        selected_top.min(max_scroll)
    } else {
        0
    };

    let paragraph = Paragraph {
        lines,
        style: body,
        scroll_y: scroll,
        framed_title: Some("wiki (Enter: open)".to_string()),
        ..Default::default()
    };
    win.paragraph(paragraph, area);
}

fn render_detail(win: &mut RenderWindow, area: Rect, content_width: usize, article: &WikiArticle) {
    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);
    let highlight = win.style(StyleKind::Highlight);
    let muted_fg = win.style(StyleKind::MutedFg);

    let dist = crate::geo::format_distance(article.dist_m);
    let coords = format!("{:.3}, {:.3}", article.lat, article.lon);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from_span(Span::styled(
        article.title.clone(),
        highlight,
    )));
    lines.push(Line::from_spans(vec![
        Span::styled(dist, muted),
        Span::styled("  ", muted),
        Span::styled(coords, muted),
    ]));
    lines.push(Line::from_span(Span::styled(
        "─".repeat(content_width),
        muted_fg,
    )));

    if article.extract.is_empty() {
        lines.push(Line::from_span(Span::styled(
            "(no summary available)",
            muted,
        )));
    } else {
        for wrapped in wrap_to_width(&article.extract, content_width) {
            lines.push(Line::from_span(Span::styled(wrapped, body)));
        }
    }

    let paragraph = Paragraph {
        lines,
        style: body,
        framed_title: Some("wiki (Esc: back)".to_string()),
        ..Default::default()
    };
    win.paragraph(paragraph, area);
}

/// Word-wrap `text` to visual cell `width` using `unicode-width` so CJK
/// characters (full-width) count correctly. Words that exceed `width` on
/// their own are placed on a line as-is rather than mid-word split.
fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for word in text.split_whitespace() {
        let word_width = word.width();
        let sep = if current.is_empty() { 0 } else { 1 };

        if current_width + sep + word_width > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }

        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}
