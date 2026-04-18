//! UI layer — ratatui layout and widgets.

pub mod layout;
pub mod theme;
pub mod widget;

use std::sync::Arc;

use theme::Theme;
use widget::Widget;
use widget::help::HelpWidget;
use widget::overlay::InfoWidget;
use widget::search::SearchWidget;
use widget::wiki::WikiWidget;

use crate::palette::Palette;
use crate::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;

/// Holds all UI widget state. Passed to layout::draw().
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
