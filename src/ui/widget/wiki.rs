//! Wiki widget — displays nearby Wikipedia articles in a side panel.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::geo::{self, LonLat};
use crate::render::frame::MapFrame;
use crate::ui::theme::Theme;
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
    /// `Some` while a detail view is open. Holds a snapshot of the article
    /// being viewed so it survives even if the candidate list is refreshed
    /// (e.g. after the map panned and new nearby articles loaded).
    detail: Option<WikiArticle>,
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
            detail: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }

    pub fn toggle(&mut self) {
        self.active = !self.active;
        self.selected = 0;
        self.detail = None;
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

        // ── Detail mode ─────────────────────────────────────────────────
        if self.detail.is_some() {
            if matches!(code, KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter) {
                self.detail = None;
                return WikiAction::None;
            }
            if up || down {
                if up {
                    self.selected = if self.selected == 0 {
                        self.articles.len() - 1
                    } else {
                        self.selected - 1
                    };
                } else {
                    self.selected = (self.selected + 1) % self.articles.len();
                }
                let article = self.articles[self.selected].clone();
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article);
                return WikiAction::JumpTo(loc);
            }
            return WikiAction::None;
        }

        // ── List mode ───────────────────────────────────────────────────
        if code == KeyCode::Enter {
            if let Some(article) = self.articles.get(self.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                self.detail = Some(article.clone());
                return WikiAction::JumpTo(loc);
            }
        } else if up || down {
            if up {
                // Wrap around: top → bottom.
                self.selected = if self.selected == 0 {
                    self.articles.len() - 1
                } else {
                    self.selected - 1
                };
            } else {
                // Wrap around: bottom → top.
                self.selected = (self.selected + 1) % self.articles.len();
            }
            let article = &self.articles[self.selected];
            return WikiAction::JumpTo(LonLat {
                lat: article.lat,
                lon: article.lon,
            });
        }

        WikiAction::None
    }

    pub fn render(&self, f: &mut Frame, map_inner: Rect, theme: &Theme) {
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

        let content_width = (panel_width as usize).saturating_sub(4).max(10);

        if let Some(ref article) = self.detail {
            self.render_detail(f, area, content_width, article, theme);
        } else {
            self.render_list(f, area, panel_height, content_width, theme);
        }
    }

    fn render_list(
        &self,
        f: &mut Frame,
        area: Rect,
        panel_height: u16,
        content_width: usize,
        theme: &Theme,
    ) {
        let block = theme.panel("wiki (Enter: open)");

        if self.articles.is_empty() {
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

        for (i, article) in self.articles.iter().enumerate() {
            let article_start = lines.len() as u16;

            if i > 0 {
                lines.push(Line::from(Span::styled(
                    &sep,
                    Style::default().fg(theme.muted_color),
                )));
            }

            let is_selected = i == self.selected;
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
        &self,
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

    /// Overlay numbered markers on the map for each wiki article.
    /// `map_area` is the terminal cell rect occupied by the rendered map.
    /// `frame` carries the center/zoom/dimensions that it was rendered at,
    /// so markers align with the displayed map regardless of any newer
    /// panning the user has done since.
    pub fn render_markers(
        &self,
        buf: &mut Buffer,
        map_area: Rect,
        frame: &MapFrame,
        theme: &Theme,
    ) {
        if !self.active || self.articles.is_empty() {
            return;
        }

        // Canvas size (pixels) that the frame was rendered at. Each terminal
        // cell is 2 pixels wide × 4 pixels tall under braille.
        let canvas_w = frame.cols as f64 * 2.0;
        let canvas_h = frame.rows as f64 * 4.0;

        let z = geo::base_zoom(frame.zoom);
        let tile_size = geo::tile_size_at_zoom(frame.zoom);
        let center_tile = geo::ll2tile(frame.center.lon, frame.center.lat, z);

        // Maximum cell coordinates within the map area we're allowed to
        // write into. Clamp to the frame size so we never draw beyond where
        // the map widget actually rendered.
        let max_col = frame.cols.min(map_area.width);
        let max_row = frame.rows.min(map_area.height);

        for (i, article) in self.articles.iter().enumerate() {
            let pt = geo::ll2tile(article.lon, article.lat, z);
            let px = canvas_w / 2.0 + (pt.x - center_tile.x) * tile_size;
            let py = canvas_h / 2.0 + (pt.y - center_tile.y) * tile_size;
            if !px.is_finite() || !py.is_finite() || px < 0.0 || py < 0.0 {
                continue;
            }

            let cell_col = (px / 2.0) as u16;
            let cell_row = (py / 4.0) as u16;
            if cell_col >= max_col || cell_row >= max_row {
                continue;
            }

            // Filled circle; the selected one gets the accent_alt colour
            // so it stands out from the rest.
            let fg = if i == self.selected {
                theme.accent_alt
            } else {
                theme.accent
            };
            let ch = '●';

            let x = map_area.x + cell_col;
            let y = map_area.y + cell_row;
            buf[(x, y)]
                .set_char(ch)
                .set_style(Style::default().fg(fg).bg(theme.bg));
        }
    }
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
