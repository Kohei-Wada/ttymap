//! Wiki widget — displays nearby Wikipedia articles in a side panel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::geo::LonLat;
use crate::ui::theme;
use crate::wikipedia::WikiArticle;

#[derive(Debug, Clone, PartialEq)]
pub enum WikiAction {
    None,
    JumpTo(LonLat),
}

pub struct WikiWidget {
    active: bool,
    articles: Vec<WikiArticle>,
    selected: usize,
    scroll: u16,
}

impl Default for WikiWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl WikiWidget {
    pub fn new() -> Self {
        Self {
            active: false,
            articles: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
        self.selected = 0;
    }

    pub fn set_articles(&mut self, new_articles: Vec<WikiArticle>) {
        let new_titles: std::collections::HashSet<String> =
            new_articles.iter().map(|a| a.title.clone()).collect();

        // Keep existing that are still in new set
        self.articles.retain(|a| new_titles.contains(&a.title));

        // Add new ones not already present
        let existing_titles: std::collections::HashSet<String> =
            self.articles.iter().map(|a| a.title.clone()).collect();
        for article in new_articles {
            if !existing_titles.contains(&article.title) {
                self.articles.push(article);
            }
        }

        if self.selected >= self.articles.len() {
            self.selected = self.articles.len().saturating_sub(1);
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> WikiAction {
        if !self.active || self.articles.is_empty() {
            return WikiAction::None;
        }

        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let up = (ctrl && matches!(code, KeyCode::Char('k') | KeyCode::Char('p')))
            || code == KeyCode::Up;
        let down = (ctrl && matches!(code, KeyCode::Char('j') | KeyCode::Char('n')))
            || code == KeyCode::Down;

        if code == KeyCode::Enter {
            if let Some(article) = self.articles.get(self.selected) {
                return WikiAction::JumpTo(LonLat {
                    lat: article.lat,
                    lon: article.lon,
                });
            }
        } else if up && self.selected > 0 {
            self.selected -= 1;
        } else if down && self.selected + 1 < self.articles.len() {
            self.selected += 1;
        }

        WikiAction::None
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect) {
        if !self.active || map_inner.width < 30 || map_inner.height < 6 {
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

        let block = theme::panel("wiki (Enter: jump)");

        if self.articles.is_empty() {
            let widget = Paragraph::new("  Loading...")
                .style(theme::muted())
                .block(block);
            f.render_widget(widget, area);
            return;
        }

        let sep = "─".repeat((panel_width as usize).saturating_sub(4));
        let mut lines: Vec<Line> = Vec::new();
        for (i, article) in self.articles.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(Span::styled(
                    &sep,
                    Style::default().fg(theme::MUTED),
                )));
            }
            let is_selected = i == self.selected;
            let dist = crate::geo::format_distance(article.dist_m);
            let title_style = if is_selected {
                Style::default().fg(theme::ACCENT_ALT)
            } else {
                theme::accent()
            };
            lines.push(Line::from(vec![
                Span::styled(&article.title, title_style),
                Span::styled(format!("  {}", dist), theme::muted()),
            ]));
            if !article.extract.is_empty() {
                let max_chars = (panel_width as usize - 4) * 2;
                let text: String = article.extract.chars().take(max_chars).collect();
                let text = if article.extract.chars().count() > max_chars {
                    format!("{}...", text)
                } else {
                    text
                };
                lines.push(Line::from(Span::styled(text, theme::text())));
            }
        }

        // Scroll to keep selected visible
        let visible_lines = panel_height.saturating_sub(2);
        let lines_per_article = 3u16;
        let selected_top = self.selected as u16 * lines_per_article;
        let scroll = if selected_top + lines_per_article > self.scroll + visible_lines {
            selected_top + lines_per_article - visible_lines
        } else if selected_top < self.scroll {
            selected_top
        } else {
            self.scroll
        };

        let widget = Paragraph::new(lines)
            .style(theme::text())
            .block(block)
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0));
        f.render_widget(widget, area);
    }
}
