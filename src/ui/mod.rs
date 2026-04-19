//! UI layer — widget state and screen rendering.

pub mod focus;
pub mod map_view;
pub mod overlay;
pub mod painter;
pub mod theme;

pub use focus::Focus;
pub use painter::MapPainter;

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use overlay::{AttributionOverlay, InfoOverlay, MapOverlay, ScaleBarOverlay};
use theme::Theme;

use crate::plugin::PluginRegistry;
use crate::plugin::help::HelpPlugin;
use crate::plugin::search::SearchPlugin;
use crate::plugin::wiki::WikiPlugin;

use crate::keymap::KeyMap;
use crate::palette::Palette;
use crate::render::frame::MapFrame;
use crate::shared::nominatim::NominatimClient;

/// Holds all UI widget state. Passed to `draw()`.
pub struct UiState {
    pub focus: Focus,
    pub widgets: PluginRegistry,
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
        let search = SearchPlugin::new(nominatim.clone());
        let mut help = HelpPlugin::new();
        let wiki = WikiPlugin::new(language, wiki_limit);

        // Help introspects the other plugins to list their activation
        // keys, so it must build after they're constructed.
        help.build(keymap, &[&search, &wiki]);

        let mut widgets = PluginRegistry::new();
        // Registration order = dispatch priority for action broadcasts.
        widgets.register(Box::new(search));
        widgets.register(Box::new(help));
        widgets.register(Box::new(wiki));

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

    let map_focused = !ui.focus.is_plugin("search");
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

        // Widgets paint world-space primitives (markers, labels, …)
        // via a single `MapPainter` exposed by the UI framework.
        {
            let mut painter = MapPainter::new(f.buffer_mut(), map_inner, map_frame, &ui.theme);
            for w in ui.widgets.iter() {
                w.paint_on_map(&mut painter);
            }
        }

        // Built-in overlays (info / attribution / scale-bar) stay as
        // typed fields: they're part of the map-viewer identity, not
        // plugin extensions.
        let attribution = AttributionOverlay {
            text: ui.attribution.as_deref().unwrap_or(""),
        };
        let overlays: [&dyn MapOverlay; 3] = [&ui.info, &ScaleBarOverlay, &attribution];
        for overlay in overlays {
            overlay.render(f.buffer_mut(), map_inner, map_frame, &ui.theme);
        }
    }

    // Render every visible plugin panel. Non-modal plugins (wiki,
    // weather, …) can stay on screen even while focus is elsewhere;
    // modal plugins (search/help) self-close on deactivate so they
    // only render while focused.
    for w in ui.widgets.iter() {
        if w.visible() {
            w.render(f, map_inner, &ui.theme);
        }
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
    if let Focus::Plugin(tag) = &ui.focus
        && let Some(w) = ui.widgets.get(tag.as_ref())
    {
        return w.footer_hints();
    }
    let mut hints = vec![
        ("hjkl", "pan"),
        ("a/z", "zoom"),
        ("/", "search"),
        ("i", "wiki"),
        ("?", "help"),
    ];
    // Tab only cycles when at least one plugin window is visible.
    if ui.widgets.iter().any(|w| w.visible()) {
        hints.push(("Tab/S-Tab", "focus"));
    }
    hints.push(("q", "quit"));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
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
        use crate::plugin::PluginCtx;
        let ui = &mut make_ui();
        assert!(ui.focus == Focus::Map);

        let mut ctx = PluginCtx {
            center: ZERO,
            focus: &mut ui.focus,
        };
        let search = ui.widgets.get_mut("search").unwrap();
        search.activate(&mut ctx);
        assert!(ctx.focus.is_plugin("search"));

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
