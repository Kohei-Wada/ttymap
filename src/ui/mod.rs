//! UI layer — widget state and screen rendering.

pub mod action;
pub mod map_view;
pub mod overlay;
pub mod palette;
pub mod router;

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use overlay::OverlayManager;

use crate::app_command::{AppCommand, Effect, FocusSurface, SurfaceCtx};
use crate::geo::LonLat;
use crate::map::render::thread::RenderHandle;
use crate::plugin::{PluginCtx, PluginRegistry};
use crate::ui::palette::CommandPalette;

use crate::color_palette::ThemeId;
use crate::focus::{Focus, FocusEvent, FocusManager};
use crate::keymap::KeyMap;
use crate::map::render::frame::MapFrame;
use crate::painter::MapPainter;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;

/// Holds all UI widget state. Passed to `draw()`.
///
/// **Theme is intentionally not here.** Theme is a cross-cutting
/// concern (UI colours + map render styler) and lives on `App` as the
/// single source of truth; UI rendering receives `&UiTheme` as a
/// parameter on `draw()`. See `docs/design.md` for the rationale.
pub struct UiState {
    pub focus: FocusManager,
    pub widgets: PluginRegistry,
    /// Command palette — builtin, not a plugin. See `ui::palette` for why.
    pub palette: CommandPalette,
    pub overlay: OverlayManager,
    pub map_frame: Option<MapFrame>,
}

impl UiState {
    pub fn new(
        nominatim: Arc<NominatimClient>,
        attribution: Option<String>,
        widgets: PluginRegistry,
    ) -> Self {
        Self {
            focus: FocusManager::new(),
            widgets,
            palette: CommandPalette::new(),
            overlay: OverlayManager::new(nominatim, attribution),
            map_frame: None,
        }
    }

    // ── Workflow methods ──────────────────────────────────────────
    //
    // Multi-step UI transitions live here (not on the controller) so
    // the invariants between `focus`, `palette`, `widgets` are owned
    // by the type that holds all three. The controller just picks
    // which workflow a given `AppCommand` maps to.

    /// Pull every frame the render thread has produced since the last
    /// tick, keeping the most recent. The receiver lives on
    /// `RenderHandle`; this method is where the UI layer reads from it.
    pub fn drain_frames(&mut self, render_handle: &RenderHandle) {
        while let Some(frame) = render_handle.try_recv_frame() {
            self.map_frame = Some(frame);
        }
    }

    /// Advance every plugin's async work by one tick. If multiple
    /// plugins produced a `AppCommand` this tick, the latest wins — only
    /// one AppCommand runs per tick to avoid cascading state changes
    /// within a single frame.
    pub fn poll_widgets(&mut self) -> Option<AppCommand> {
        let mut async_cmd: Option<AppCommand> = None;
        for w in self.widgets.iter_mut() {
            w.poll();
            if let Some(cmd) = w.pending_command() {
                async_cmd = Some(cmd);
            }
        }
        async_cmd
    }

    /// Open the command palette with its default provider. Palette
    /// becomes visible; the focus manager records the claim. The
    /// current `theme_id` is taken as a parameter (theme lives on
    /// `App`, not `UiState`) so the palette can highlight the active
    /// entry in its theme picker.
    pub fn open_palette(&mut self, keymap: &KeyMap, theme_id: ThemeId) {
        self.palette.activate(&self.widgets, keymap, theme_id);
        self.focus.on(
            FocusEvent::Claimed(palette::SURFACE_ID.into()),
            &mut self.widgets,
        );
    }

    /// Cycle keyboard focus across visible plugins. `true` = forward
    /// (Tab), `false` = backward (Shift-Tab). Returns whether the
    /// focus actually moved. Tab is an explicit user intent, not a
    /// reactive state change, so it stays a dedicated method rather
    /// than riding `FocusEvent`.
    pub fn cycle_focus(&mut self, forward: bool) -> bool {
        self.focus.cycle(&mut self.widgets, forward)
    }

    /// Drive a plugin through an activation request (activation key,
    /// palette selection, or external command). Re-activating a
    /// currently-focused plugin toggles it off. For first-time
    /// activation, brings the plugin to the front and calls its
    /// `activate` hook. The `wants_focus` gate lives here (not in
    /// the focus manager) — we only emit `Claimed` if the plugin
    /// actually asks for focus, so headless plugins (like `here`)
    /// don't steal it.
    pub fn activate_plugin(&mut self, tag: &str, center: LonLat) {
        // Toggle-off: re-activating the currently-focused plugin closes it.
        if self.focus.is_modal(tag) {
            if let Some(w) = self.widgets.get_mut(tag) {
                w.close();
            }
            self.focus
                .on(FocusEvent::Released(tag.to_string().into()), &mut self.widgets);
            return;
        }

        // Normal activation.
        self.widgets.bring_to_front(tag);
        let mut ctx = PluginCtx { center };
        let wants_focus = if let Some(w) = self.widgets.get_mut(tag) {
            w.activate(&mut ctx);
            w.wants_focus()
        } else {
            return;
        };
        if wants_focus {
            self.focus.on(
                FocusEvent::Claimed(tag.to_string().into()),
                &mut self.widgets,
            );
        }
    }

    /// Hand a key to the currently-focused surface and apply the
    /// auto-release invariant (palette/plugin closing during
    /// `handle_key` drops focus). Returns the surface's `Effect`, or
    /// `None` when no surface has focus — the router then falls
    /// through to the [`BackgroundResponder`].
    pub fn deliver_to_focused_surface(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: SurfaceCtx,
    ) -> Option<Effect> {
        let id = match self.focus.current().clone() {
            Focus::Background => return None,
            Focus::Modal(id) => id,
        };

        // Palette is not in the plugin registry — special-case its
        // delivery. Every other modal id is looked up as a plugin.
        let (effect, still_visible) = if id == palette::SURFACE_ID {
            // Inherent and trait both have a `handle_key`; spell out
            // the trait so the SurfaceCtx-taking impl is selected.
            let effect = <CommandPalette as FocusSurface>::handle_key(
                &mut self.palette,
                code,
                modifiers,
                ctx,
            );
            (effect, self.palette.is_visible())
        } else {
            let effect = match self.widgets.get_mut(id.as_ref()) {
                Some(p) => crate::plugin::deliver(p, code, modifiers, ctx),
                None => Effect::Pass,
            };
            let still_visible = self
                .widgets
                .get(id.as_ref())
                .is_some_and(|w| w.visible());
            (effect, still_visible)
        };

        if !still_visible {
            self.focus
                .on(FocusEvent::Released(id), &mut self.widgets);
        }
        Some(effect)
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
            for w in ui.widgets.iter() {
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
    for w in ui.widgets.iter() {
        if w.visible() {
            w.render(f, map_inner, theme);
        }
    }

    // Palette draws on top of every plugin when visible (it's modal
    // and coordinates over them).
    if ui.palette.is_visible() {
        ui.palette.render(f, map_inner, theme);
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
            return ui.palette.footer_hints();
        }
        if let Some(w) = ui.widgets.get(id.as_ref()) {
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
    use crate::map::render::frame::{MapCell, MapFrame};
    use crossterm::event::{KeyCode, KeyModifiers};

    const ZERO: LonLat = LonLat { lon: 0.0, lat: 0.0 };

    fn make_ui() -> UiState {
        use crate::plugin::search::SearchPlugin;
        let nominatim = Arc::new(NominatimClient::new());
        let mut widgets = PluginRegistry::new();
        widgets.register(Box::new(SearchPlugin::new(nominatim.clone())));
        UiState::new(nominatim, None, widgets)
    }

    #[test]
    fn test_ui_state_initial() {
        let ui = make_ui();
        assert_eq!(ui.focus.current(), &Focus::Background);
        assert!(ui.map_frame.is_none());
    }

    #[test]
    fn test_ui_state_search_lifecycle() {
        use crate::plugin::PluginCtx;
        let ui = &mut make_ui();
        assert_eq!(ui.focus.current(), &Focus::Background);

        // Plugin.activate / handle_key no longer touch focus — the
        // host (`ui::router::activate_plugin` + the focused-plugin
        // dispatch loop) owns every focus transition. Here we just
        // verify the plugin's own state machine: open on activate,
        // close on Esc.
        let mut ctx = PluginCtx { center: ZERO };
        let search = ui.widgets.get_mut("search").unwrap();
        search.activate(&mut ctx);
        assert!(search.visible());
        assert!(search.wants_focus());

        search.handle_key(KeyCode::Char('a'), KeyModifiers::NONE, &mut ctx);
        search.handle_key(KeyCode::Esc, KeyModifiers::NONE, &mut ctx);
        assert!(!search.visible());
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
