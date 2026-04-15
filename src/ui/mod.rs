//! UI layer — ratatui layout and widgets.

pub mod layout;
pub mod theme;
pub mod widget;

use widget::help::HelpWidget;
use widget::info::InfoWidget;
use widget::search::SearchWidget;
use widget::wiki::WikiWidget;

use crate::render::frame::MapFrame;

/// Holds all UI widget state. Passed to layout::draw().
impl Default for UiState {
    fn default() -> Self { Self::new() }
}

pub struct UiState {
    pub search: SearchWidget,
    pub info: InfoWidget,
    pub help: HelpWidget,
    pub wiki: WikiWidget,
    pub map_frame: Option<MapFrame>,
}

impl UiState {
    pub fn new() -> Self {
        Self {
            search: SearchWidget::new(),
            info: InfoWidget::new(),
            help: HelpWidget::new(),
            wiki: WikiWidget::new(),
            map_frame: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use crate::render::frame::{MapCell, MapFrame};

    #[test]
    fn test_ui_state_initial() {
        let ui = UiState::new();
        assert!(!ui.search.is_active());
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn test_ui_state_search_lifecycle() {
        let ui = &mut UiState::new();
        assert!(!ui.search.is_active());

        ui.search.open();
        assert!(ui.search.is_active());

        ui.search.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        ui.search.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!ui.search.is_active());
    }

    #[test]
    fn test_ui_state_map_frame() {
        let mut ui = UiState::new();
        assert!(ui.map_frame.is_none());

        ui.map_frame = Some(MapFrame {
            cells: vec![MapCell { ch: ' ', fg: 0, bg: 0 }],
            cols: 1,
            rows: 1,
        });
        assert!(ui.map_frame.is_some());
    }
}
