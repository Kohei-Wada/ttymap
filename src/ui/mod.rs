//! UI layer — widget state and screen rendering.

pub mod theme;
pub mod widget;

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use theme::Theme;
use widget::Widget;
use widget::help::HelpWidget;
use widget::overlay::{
    AttributionOverlay, InfoWidget, MapOverlay, MarkersOverlay, ScaleBarOverlay,
};
use widget::search::{self, SearchWidget};
use widget::wiki::{self, WikiWidget};

use crate::palette::Palette;
use crate::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;

/// Holds all UI widget state. Passed to `draw()`.
pub struct UiState {
    pub search: SearchWidget,
    pub info: InfoWidget,
    pub help: HelpWidget,
    pub wiki: WikiWidget,
    pub map_frame: Option<MapFrame>,
    pub theme: Theme,
    pub attribution: Option<String>,
}

impl UiState {
    pub fn new(
        palette: &Palette,
        language: &str,
        wiki_limit: u32,
        nominatim: Arc<NominatimClient>,
        attribution: Option<String>,
    ) -> Self {
        Self {
            search: SearchWidget::new(nominatim.clone()),
            info: InfoWidget::new(nominatim),
            help: HelpWidget::new(),
            wiki: WikiWidget::new(language, wiki_limit),
            map_frame: None,
            theme: Theme::from_palette(palette),
            attribution,
        }
    }

    /// Interactive widgets in priority order. `app.rs` uses this to
    /// dispatch key / action events without hard-coding per-widget
    /// names. Search takes precedence (modal), then help (modal), then
    /// wiki (non-modal — falls through unrecognised keys).
    pub fn widgets_mut(&mut self) -> [&mut dyn Widget; 3] {
        [&mut self.search, &mut self.help, &mut self.wiki]
    }
}

/// Draw the full screen. `app.rs` delegates all rendering here.
pub fn draw(f: &mut Frame, ui: &UiState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

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

        // Map overlays — each stamps on top of the rendered map. Adding
        // a new overlay means implementing MapOverlay and appending here.
        let wiki_points = wiki::marker_points(&ui.wiki, &ui.theme);
        let wiki_markers = MarkersOverlay {
            points: &wiki_points,
        };
        let attribution = AttributionOverlay {
            text: ui.attribution.as_deref().unwrap_or(""),
        };
        let overlays: [&dyn MapOverlay; 4] =
            [&wiki_markers, &ui.info, &ScaleBarOverlay, &attribution];
        for overlay in overlays {
            overlay.render(f.buffer_mut(), map_inner, map_frame, &ui.theme);
        }
    }

    wiki::render_panel(&ui.wiki, f, map_inner, &ui.theme);
    search::render_panel(&ui.search, f, map_inner, &ui.theme);
    ui.help.render(f, map_inner, &ui.theme);

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
        if ui.wiki.is_detail_open() {
            vec![
                ("C-n/C-p", "prev/next"),
                ("Enter/Esc", "back"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("?", "help"),
            ]
        } else {
            vec![
                ("C-n/C-p", "select"),
                ("Enter", "open"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("/", "search"),
                ("?", "help"),
            ]
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::input::Action;
    use crate::geo::LonLat;
    use crate::render::frame::{MapCell, MapFrame};
    use crossterm::event::{KeyCode, KeyModifiers};

    const ZERO: LonLat = LonLat { lon: 0.0, lat: 0.0 };

    #[test]
    fn test_ui_state_initial() {
        let ui = UiState::new(
            &crate::palette::DARK,
            "en",
            5,
            Arc::new(NominatimClient::new()),
            None,
        );
        assert!(!ui.search.is_active());
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn test_ui_state_search_lifecycle() {
        let ui = &mut UiState::new(
            &crate::palette::DARK,
            "en",
            5,
            Arc::new(NominatimClient::new()),
            None,
        );
        assert!(!ui.search.is_active());

        assert!(ui.search.handle_action(&Action::SearchOpen, ZERO));
        assert!(ui.search.is_active());

        ui.search
            .handle_key(KeyCode::Char('a'), KeyModifiers::NONE, ZERO);
        ui.search.handle_key(KeyCode::Esc, KeyModifiers::NONE, ZERO);
        assert!(!ui.search.is_active());
    }

    #[test]
    fn test_ui_state_map_frame() {
        let mut ui = UiState::new(
            &crate::palette::DARK,
            "en",
            5,
            Arc::new(NominatimClient::new()),
            None,
        );
        assert!(ui.map_frame.is_none());

        ui.map_frame = Some(MapFrame {
            cells: vec![MapCell {
                ch: ' ',
                fg: 0,
                bg: 0,
            }],
            cols: 1,
            rows: 1,
            center: crate::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        });
        assert!(ui.map_frame.is_some());
    }
}
