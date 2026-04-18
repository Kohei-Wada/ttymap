//! UI layer — ratatui layout and widgets.

pub mod layout;
pub mod theme;
pub mod widget;

use theme::Theme;
use widget::help::HelpWidget;
use widget::info::InfoWidget;
use widget::search::SearchWidget;
use widget::wiki::WikiWidget;

use crate::palette::Palette;
use crate::render::frame::MapFrame;

/// Holds all UI widget state. Passed to layout::draw().
pub struct UiState {
    pub search: SearchWidget,
    pub info: InfoWidget,
    pub help: HelpWidget,
    pub wiki: WikiWidget,
    pub map_frame: Option<MapFrame>,
    pub theme: Theme,
}

impl UiState {
    pub fn new(palette: &Palette) -> Self {
        Self {
            search: SearchWidget::new(),
            info: InfoWidget::new(),
            help: HelpWidget::new(),
            wiki: WikiWidget::new(),
            map_frame: None,
            theme: Theme::from_palette(palette),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::frame::{MapCell, MapFrame};
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn test_ui_state_initial() {
        let ui = UiState::new(&crate::palette::DARK);
        assert!(!ui.search.is_active());
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn test_ui_state_search_lifecycle() {
        let ui = &mut UiState::new(&crate::palette::DARK);
        assert!(!ui.search.is_active());

        ui.search.open();
        assert!(ui.search.is_active());

        ui.search.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        ui.search.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!ui.search.is_active());
    }

    #[test]
    fn test_ui_state_map_frame() {
        let mut ui = UiState::new(&crate::palette::DARK);
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
