//! UI layer — widget state and screen rendering.

pub mod overlay;

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use overlay::OverlayManager;

use crate::compositor::MapPainter;
use crate::compositor::{Compositor, Context};
use crate::map::render::frame::MapFrame;
use crate::map::render::thread::RenderHandle;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;

/// Thin container for UI-level state. Owns:
/// - [`OverlayManager`] (info / attribution / scale-bar)
/// - the latest map frame snapshot
///
/// Focus / modal state lives on the [`Compositor`] owned by `App`;
/// UiState forwards rendering through it via [`draw`].
///
/// **Theme is intentionally not here.** Theme is a cross-cutting
/// concern (UI colours + map render styler) and lives on `App` as the
/// single source of truth; UI rendering receives `&UiTheme` as a
/// parameter on `draw()`. See `docs/design.md` for the rationale.
pub struct UiState {
    pub overlay: OverlayManager,
    pub map_frame: Option<MapFrame>,
}

impl UiState {
    pub fn new(nominatim: Arc<NominatimClient>, attribution: Option<String>) -> Self {
        Self {
            overlay: OverlayManager::new(nominatim, attribution),
            map_frame: None,
        }
    }

    pub fn drain_frames(&mut self, render_handle: &RenderHandle) {
        while let Some(frame) = render_handle.try_recv_frame() {
            self.map_frame = Some(frame);
        }
    }
}

/// Draw the full screen. App passes the compositor so world-space
/// overlays (wiki markers via `Component::paint_on_map`) and on-top
/// panels (via `Component::render`) go through the same draw pass
/// as the map.
pub fn draw(f: &mut Frame, ui: &UiState, compositor: &Compositor, theme: &UiTheme, ctx: &Context) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

    let map_block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(" world ");
    let map_inner = map_block.inner(map_area);
    f.render_widget(map_block, map_area);
    if let Some(ref map_frame) = ui.map_frame {
        f.render_widget(map_frame, map_inner);

        // World-space overlays from components on the compositor
        // stack (wiki markers etc.). Focus-gated: closing the panel
        // drops the component, which drops the paint hook.
        {
            let mut painter = MapPainter::new(f.buffer_mut(), map_inner, map_frame, theme);
            compositor.paint_on_map(&mut painter);
        }

        ui.overlay
            .render(f.buffer_mut(), map_inner, map_frame, theme);
    }

    // Modal panels on top of the map, bottom-up.
    compositor.render(f, map_inner, theme, ctx);

    let hints = build_hints(compositor);
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

fn build_hints(compositor: &Compositor) -> Vec<(&'static str, &'static str)> {
    let mut hints = compositor.footer_hints();
    // Cycle hint: meaningful whenever there's more than just the
    // base layer on the stack — Tab toggles focus between the base
    // and any modal(s), including the single-modal case.
    if compositor.len() > 1 {
        let cycle_hint = ("Tab/S-Tab", "focus");
        match hints.iter().position(|(k, _)| *k == "q") {
            Some(i) => hints.insert(i, cycle_hint),
            None => hints.push(cycle_hint),
        }
    }
    hints
}
