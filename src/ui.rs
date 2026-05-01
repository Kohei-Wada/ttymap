//! UI layer — the single screen-rendering pass.
//!
//! No state of its own. App owns the latest [`MapFrame`] (drained
//! from the render thread each tick) and passes it through to
//! [`draw`] alongside the compositor. Built-in chrome
//! (info / scale_bar / attribution) all migrated to plugins, so this
//! module is now a thin layout + draw routine.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::compositor::{Compositor, Context, MapApi};
use crate::lua::LuaTickRegistry;
use crate::map::render::frame::MapFrame;
use crate::map::render::overlay::UserPolyline;
use crate::theme::UiTheme;

/// Draw the full screen. Caller passes the latest map snapshot
/// (or `None` if the render thread hasn't produced one yet) plus
/// the compositor; world-space overlays (wiki markers via
/// `Component::paint_on_map`) and on-top panels
/// (via `Component::render`) go through the same draw pass as the
/// map.
pub fn draw(
    f: &mut Frame,
    map_frame: Option<&MapFrame>,
    compositor: &Compositor,
    tick_registry: &LuaTickRegistry,
    theme: &UiTheme,
    ctx: &Context,
    overlay_sink: &mut Vec<UserPolyline>,
) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let map_area = chunks[0];
    let footer_area = chunks[1];

    let map_block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(" world ");
    let map_inner = map_block.inner(map_area);
    f.render_widget(map_block, map_area);
    if let Some(map_frame) = map_frame {
        f.render_widget(map_frame, map_inner);

        // World-space overlays + always-on chrome from components on
        // the compositor (wiki markers, info bar, scale, attribution).
        // Focus-gated: closing a panel drops the component which drops
        // its paint hook.
        let mut api = MapApi::new(
            f.buffer_mut(),
            map_inner,
            map_frame,
            theme,
            ctx.cursor,
            overlay_sink,
        );
        // Tick every plugin-declared `on_tick` callback against the
        // live MapApi *before* the compositor paints — so any
        // plugin-emitted points lie underneath focused-component
        // overlays in the same frame, matching the layering plugin
        // authors expect from `Component::paint_on_map`.
        tick_registry.tick(&mut api);
        compositor.paint_on_map(&mut api);
    }

    // Modal panels on top of the map, bottom-up.
    compositor.render(f, map_inner, theme, ctx);

    let hints = build_hints(compositor);
    let sep = Span::styled("  ", Style::default().fg(theme.muted_color));
    let mut spans: Vec<Span> = Vec::new();

    // Lead with the focused component's name (e.g. "[wiki]") so the
    // user can tell which plugin is consuming keystrokes when modals
    // stack. Empty for the base layer — no chrome when focus is on
    // the map itself.
    let focused = compositor.focused_name();
    if !focused.is_empty() {
        spans.push(Span::styled(
            format!(" {} ", focused),
            Style::default().fg(theme.bg).bg(theme.accent_alt),
        ));
        spans.push(sep.clone());
    }

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
