//! UI layer — widget state and screen rendering.

pub mod focus;
pub mod theme;
pub mod widget;

pub use focus::Focus;

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use theme::Theme;
use widget::WidgetRegistry;
use widget::help::HelpWidget;
use widget::overlay::{
    AttributionOverlay, InfoOverlay, MapOverlay, MarkersOverlay, ScaleBarOverlay,
};
use widget::search::SearchWidget;
use widget::wiki::WikiWidget;

use crate::keymap::KeyMap;
use crate::palette::Palette;
use crate::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;

/// Holds all UI widget state. Passed to `draw()`.
pub struct UiState {
    pub focus: Focus,
    pub widgets: WidgetRegistry,
    pub info: InfoOverlay,
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
        keymap: &KeyMap,
    ) -> Self {
        let mut help = HelpWidget::new();
        help.build(keymap);

        let mut widgets = WidgetRegistry::new();
        // Registration order = dispatch priority for action broadcasts.
        widgets.register(Box::new(SearchWidget::new(nominatim.clone())));
        widgets.register(Box::new(help));
        widgets.register(Box::new(WikiWidget::new(language, wiki_limit)));

        Self {
            focus: Focus::Map,
            widgets,
            info: InfoOverlay::new(nominatim),
            map_frame: None,
            theme: Theme::from_palette(palette),
            attribution,
        }
    }
}

/// Draw the full screen. `app.rs` delegates all rendering here.
pub fn draw(f: &mut Frame, ui: &UiState) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

    let map_focused = !ui.focus.is_widget("search");
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

        // Gather marker points from every widget that supplies any.
        let widget_markers: Vec<_> = ui
            .widgets
            .iter()
            .flat_map(|w| w.markers(&ui.theme))
            .collect();
        let markers = MarkersOverlay {
            points: &widget_markers,
        };
        let attribution = AttributionOverlay {
            text: ui.attribution.as_deref().unwrap_or(""),
        };
        let overlays: [&dyn MapOverlay; 4] = [&markers, &ui.info, &ScaleBarOverlay, &attribution];
        for overlay in overlays {
            overlay.render(f.buffer_mut(), map_inner, map_frame, &ui.theme);
        }
    }

    // Modal panels render only while focused. This keeps focus the
    // single source of truth — if a widget isn't focused, it isn't on
    // screen regardless of any lingering internal state.
    if let Focus::Widget(tag) = &ui.focus
        && let Some(w) = ui.widgets.get(tag.as_ref())
    {
        w.render(f, map_inner, &ui.theme);
    }

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
    // Focused widget provides its own context-sensitive hints.
    if let Focus::Widget(tag) = &ui.focus
        && let Some(w) = ui.widgets.get(tag.as_ref())
    {
        return w.footer_hints();
    }
    {
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
    use crate::core::Action;
    use crate::geo::LonLat;
    use crate::render::frame::{MapCell, MapFrame};
    use crossterm::event::{KeyCode, KeyModifiers};

    const ZERO: LonLat = LonLat { lon: 0.0, lat: 0.0 };

    fn make_ui() -> UiState {
        let keymap = KeyMap::default();
        UiState::new(
            &crate::palette::DARK,
            "en",
            5,
            Arc::new(NominatimClient::new()),
            None,
            &keymap,
        )
    }

    #[test]
    fn test_ui_state_initial() {
        let ui = make_ui();
        assert!(ui.focus == Focus::Map);
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn test_ui_state_search_lifecycle() {
        use crate::ui::widget::WidgetCtx;
        let ui = &mut make_ui();
        assert!(ui.focus == Focus::Map);

        let mut ctx = WidgetCtx {
            center: ZERO,
            focus: &mut ui.focus,
        };
        let search = ui.widgets.get_mut("search").unwrap();
        assert!(search.handle_action(&Action::SearchOpen, &mut ctx));
        assert!(ctx.focus.is_widget("search"));

        search.handle_key(KeyCode::Char('a'), KeyModifiers::NONE, &mut ctx);
        search.handle_key(KeyCode::Esc, KeyModifiers::NONE, &mut ctx);
        assert!(matches!(*ctx.focus, Focus::Map));
    }

    #[test]
    fn test_ui_state_map_frame() {
        let mut ui = make_ui();
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
