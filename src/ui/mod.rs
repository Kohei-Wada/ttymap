//! UI layer — widget state and screen rendering.

pub mod action;
pub mod map_view;
pub mod overlay;
pub mod palette;
pub mod router;

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use overlay::OverlayManager;

use crate::app_command::FocusSurface;
use crate::focus::{Focus, FocusManager};
use crate::map::render::frame::MapFrame;
use crate::map::render::thread::RenderHandle;
use crate::painter::MapPainter;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;

/// Thin container for UI-level state. Owns:
/// - [`FocusManager`] (the focus state and every focusable surface — palette + plugins)
/// - [`OverlayManager`] (info / attribution / scale-bar)
/// - the latest map frame snapshot
///
/// All focus / palette / plugin workflows live on `FocusManager`;
/// `UiState` holds the references and forwards `drain_frames`.
///
/// **Theme is intentionally not here.** Theme is a cross-cutting
/// concern (UI colours + map render styler) and lives on `App` as the
/// single source of truth; UI rendering receives `&UiTheme` as a
/// parameter on `draw()`. See `docs/design.md` for the rationale.
pub struct UiState {
    pub focus: FocusManager,
    pub overlay: OverlayManager,
    pub map_frame: Option<MapFrame>,
}

impl UiState {
    pub fn new(
        nominatim: Arc<NominatimClient>,
        attribution: Option<String>,
        focus: FocusManager,
    ) -> Self {
        Self {
            focus,
            overlay: OverlayManager::new(nominatim, attribution),
            map_frame: None,
        }
    }

    /// Pull every frame the render thread has produced since the last
    /// tick, keeping the most recent. The receiver lives on
    /// `RenderHandle`; this method is where the UI layer reads from it.
    pub fn drain_frames(&mut self, render_handle: &RenderHandle) {
        while let Some(frame) = render_handle.try_recv_frame() {
            self.map_frame = Some(frame);
        }
    }
}

/// Draw the full screen. `app.rs` delegates all rendering here and
/// passes the active `UiTheme` (owned by `App`).
pub fn draw(f: &mut Frame, ui: &UiState, theme: &UiTheme) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

    let map_focused = !ui.focus.is_modal("search");
    let border_color = if map_focused {
        theme.accent
    } else {
        theme.muted_color
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
            let mut painter = MapPainter::new(f.buffer_mut(), map_inner, map_frame, theme);
            for w in ui.focus.widgets().iter() {
                w.paint_on_map(&mut painter);
            }
        }

        // Built-in overlays (info / attribution / scale-bar). The
        // manager owns their state and paint order so the caller
        // doesn't distinguish between them.
        ui.overlay
            .render(f.buffer_mut(), map_inner, map_frame, theme);
    }

    // Render every visible plugin panel. Non-modal plugins (wiki,
    // weather, …) can stay on screen even while focus is elsewhere;
    // modal plugins (search/help) self-close on deactivate so they
    // only render while focused.
    for w in ui.focus.widgets().iter() {
        if w.is_visible() {
            w.render(f, map_inner, theme);
        }
    }

    // Palette draws on top of every plugin when visible (it's modal
    // and coordinates over them).
    if ui.focus.palette().is_visible() {
        ui.focus.palette().render(f, map_inner, theme);
    }

    let hints = build_hints(ui);
    let sep = Span::styled("  ", Style::default().fg(theme.muted_color));
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(
            format!(" {} ", key),
            Style::default().fg(theme.bg).bg(theme.accent),
        ));
        spans.push(Span::styled(
            format!(" {}", desc),
            Style::default().fg(theme.muted_color),
        ));
    }
    let footer = Paragraph::new(Line::from(spans));
    f.render_widget(footer, footer_area);
}

fn build_hints(ui: &UiState) -> Vec<(&'static str, &'static str)> {
    // Focused surface provides its own context-sensitive hints.
    if let Focus::Modal(id) = ui.focus.current() {
        if id == palette::SURFACE_ID {
            return ui.focus.palette().footer_hints();
        }
        if let Some(w) = ui.focus.widgets().get(id.as_ref()) {
            return w.footer_hints();
        }
    }
    let mut hints = vec![
        ("hjkl", "pan"),
        ("a/z", "zoom"),
        (":", "cmd"),
        ("/", "search"),
        ("i", "wiki"),
        ("?", "help"),
    ];
    // Tab only cycles when at least one plugin window is visible.
    if ui.focus.widgets().iter().any(|w| w.is_visible()) {
        hints.push(("Tab/S-Tab", "focus"));
    }
    hints.push(("q", "quit"));
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LonLat;
    use crate::map::render::frame::{MapCell, MapFrame};
    use crate::plugin::PluginRegistry;

    fn make_ui() -> UiState {
        use crate::plugin::search::SearchPlugin;
        use crate::ui::palette::CommandPalette;
        let nominatim = Arc::new(NominatimClient::new());
        let mut widgets = PluginRegistry::new();
        widgets.register(Box::new(SearchPlugin::new(nominatim.clone())));
        let focus = FocusManager::new(CommandPalette::new(), widgets);
        UiState::new(nominatim, None, focus)
    }

    #[test]
    fn test_ui_state_initial() {
        let ui = make_ui();
        assert_eq!(ui.focus.current(), &Focus::Background);
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn search_plugin_open_then_close_on_esc() {
        use crate::app_command::SurfaceCtx;
        use crate::plugin::Plugin;
        use crate::plugin::search::SearchPlugin;
        use crossterm::event::{KeyCode, KeyModifiers};

        const ZERO: LonLat = LonLat { lon: 0.0, lat: 0.0 };

        // Plugin.activate / handle_key never touch focus — the host
        // (`FocusManager::activate_plugin` + the focused-surface
        // delivery loop) owns every focus transition. Verify just the
        // plugin's own state machine: open on activate, close on Esc.
        let nominatim = Arc::new(NominatimClient::new());
        let mut search = SearchPlugin::new(nominatim);
        let ctx = SurfaceCtx { center: ZERO };

        search.activate(ctx);
        assert!(search.is_visible());
        assert!(search.wants_focus());

        search.handle_key(KeyCode::Char('a'), KeyModifiers::NONE, ctx);
        search.handle_key(KeyCode::Esc, KeyModifiers::NONE, ctx);
        assert!(!search.is_visible());
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
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        });
        assert!(ui.map_frame.is_some());
    }
}
