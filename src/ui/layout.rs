//! Screen layout — arranges widgets into the terminal frame.
//! app.rs delegates all drawing to this module.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::UiState;

/// Draw the full screen.
pub fn draw(f: &mut Frame, ui: &UiState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

    // Map area with border
    let map_focused = !ui.search.is_active();
    let border_color = if map_focused {
        ui.theme.accent
    } else {
        ui.theme.muted_color
    };
    let map_block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" world ");
    let map_inner = map_block.inner(map_area);
    f.render_widget(map_block, map_area);
    if let Some(ref map_frame) = ui.map_frame {
        f.render_widget(map_frame, map_inner);
    }

    // Info overlay
    ui.info.render(f, map_inner, &ui.theme);

    // Wiki panel
    ui.wiki.render(f, map_inner, &ui.theme);

    // Search overlay
    ui.search.render(f, map_inner, &ui.theme);

    // Help overlay
    ui.help.render(f, map_inner, &ui.theme);

    // Footer: context-sensitive key hints
    let hints = build_hints(ui);
    let sep = Span::styled("  ", Style::default().fg(ui.theme.muted_color));
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(
            format!(" {} ", key),
            Style::default().fg(ui.theme.bg).bg(ui.theme.accent),
        ));
        spans.push(Span::styled(
            format!(" {}", desc),
            Style::default().fg(ui.theme.muted_color),
        ));
    }
    let footer = Paragraph::new(Line::from(spans));
    f.render_widget(footer, footer_area);
}

fn build_hints(ui: &UiState) -> Vec<(&'static str, &'static str)> {
    if ui.search.is_active() {
        if ui.search.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    } else if ui.help.is_active() {
        vec![("any key", "close")]
    } else if ui.wiki.is_active() {
        vec![
            ("C-n/C-p", "select"),
            ("Enter", "jump"),
            ("i", "close wiki"),
            ("/", "search"),
            ("?", "help"),
        ]
    } else {
        vec![
            ("hjkl", "pan"),
            ("a/z", "zoom"),
            ("/", "search"),
            ("i", "wiki"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }
}
