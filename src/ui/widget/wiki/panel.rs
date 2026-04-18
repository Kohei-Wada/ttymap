//! Wiki side panel — list view and detail view.
//!
//! Stateless renderer; reads `WikiState` and draws into the ratatui frame.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ui::theme::Theme;
use crate::wikipedia::WikiArticle;

use super::state::WikiState;

/// Render the wiki side panel (list or detail view) if active.
pub fn render_panel(state: &WikiState, f: &mut Frame, map_inner: Rect, theme: &Theme) {
    if !state.active || map_inner.width < 30 || map_inner.height < 6 {
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
    f.render_widget(Clear, area);

    let content_width = (panel_width as usize).saturating_sub(4).max(10);

    if let Some(ref article) = state.detail {
        render_detail(f, area, content_width, article, theme);
    } else {
        render_list(state, f, area, panel_height, content_width, theme);
    }
}

fn render_list(
    state: &WikiState,
    f: &mut Frame,
    area: Rect,
    panel_height: u16,
    content_width: usize,
    theme: &Theme,
) {
    let block = theme.panel("wiki (Enter: open)");

    if state.articles.is_empty() {
        let widget = Paragraph::new("  Loading...")
            .style(theme.muted())
            .block(block);
        f.render_widget(widget, area);
        return;
    }

    let sep = "─".repeat(content_width);
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_top: u16 = 0;
    let mut selected_height: u16 = 1;

    for (i, article) in state.articles.iter().enumerate() {
        let article_start = lines.len() as u16;

        if i > 0 {
            lines.push(Line::from(Span::styled(
                &sep,
                Style::default().fg(theme.muted_color),
            )));
        }

        let is_selected = i == state.selected;
        let dist = crate::geo::format_distance(article.dist_m);
        let title_style = if is_selected {
            Style::default().fg(theme.accent_alt)
        } else {
            theme.accent_style()
        };
        lines.push(Line::from(vec![
            Span::styled(&article.title, title_style),
            Span::styled(format!("  {}", dist), theme.muted()),
        ]));

        if !article.extract.is_empty() {
            // Cap the extract at roughly two lines of content, then wrap
            // manually so scroll math below can treat each pushed Line as
            // one output row (Paragraph::wrap is not used any more).
            let max_chars = content_width * 2;
            let raw: String = article.extract.chars().take(max_chars).collect();
            let truncated = if article.extract.chars().count() > max_chars {
                format!("{}...", raw)
            } else {
                raw
            };
            for wrapped in wrap_to_width(&truncated, content_width) {
                lines.push(Line::from(Span::styled(wrapped, theme.text())));
            }
        }

        if is_selected {
            selected_top = article_start;
            selected_height = (lines.len() as u16).saturating_sub(article_start).max(1);
        }
    }

    // Scroll to keep the selected article visible. With wrap disabled on
    // Paragraph, each Line above corresponds exactly to one output row,
    // so this math is precise.
    let visible_lines = panel_height.saturating_sub(2);
    let scroll = (selected_top + selected_height).saturating_sub(visible_lines);

    let widget = Paragraph::new(lines)
        .style(theme.text())
        .block(block)
        .scroll((scroll, 0));
    f.render_widget(widget, area);
}

fn render_detail(
    f: &mut Frame,
    area: Rect,
    content_width: usize,
    article: &WikiArticle,
    theme: &Theme,
) {
    let block = theme.panel("wiki (Esc: back)");
    let dist = crate::geo::format_distance(article.dist_m);
    let coords = format!("{:.3}, {:.3}", article.lat, article.lon);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        &article.title,
        Style::default().fg(theme.accent_alt),
    )));
    lines.push(Line::from(vec![
        Span::styled(dist, theme.muted()),
        Span::styled("  ", theme.muted()),
        Span::styled(coords, theme.muted()),
    ]));
    lines.push(Line::from(Span::styled(
        "─".repeat(content_width),
        Style::default().fg(theme.muted_color),
    )));

    if article.extract.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no summary available)",
            theme.muted(),
        )));
    } else {
        for wrapped in wrap_to_width(&article.extract, content_width) {
            lines.push(Line::from(Span::styled(wrapped, theme.text())));
        }
    }

    let widget = Paragraph::new(lines).style(theme.text()).block(block);
    f.render_widget(widget, area);
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
