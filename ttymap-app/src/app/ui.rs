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

use crate::compositor::{Compositor, Context};
use crate::lua::{LuaHandle, MapApi};
use crate::theme::UiTheme;
use ttymap_engine::map::render::frame::MapFrame;
use ttymap_engine::map::render::overlay::UserPolyline;

/// Draw the full screen. Caller passes the latest map snapshot
/// (or `None` if the render thread hasn't produced one yet) plus
/// the compositor; world-space overlays (wiki markers etc., painted
/// by Lua plugins through `lua::tick::dispatch_tick`) and on-top
/// panels (via `Component::render`) go through the same draw pass
/// as the map.
/// Per-frame inputs collected by [`App::render_into`]. Bundled
/// to keep [`draw`] under clippy's argument-count threshold and to
/// give related fields a single place to grow.
pub struct DrawInputs<'a> {
    pub map_frame: Option<&'a MapFrame>,
    pub compositor: &'a Compositor,
    pub lua: &'a LuaHandle,
    pub theme: &'a UiTheme,
    pub ctx: &'a Context,
    pub overlay_sink: &'a mut Vec<UserPolyline>,
    pub sidebar_open: bool,
    pub sidebar_width: u16,
}

pub fn draw(f: &mut Frame, inputs: DrawInputs<'_>) {
    let DrawInputs {
        map_frame,
        compositor,
        lua,
        theme,
        ctx,
        overlay_sink,
        sidebar_open,
        sidebar_width,
    } = inputs;
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());

    let main_area = chunks[0];
    let footer_area = chunks[1];

    // Left sidebar (when toggled on) takes a fixed width; remaining
    // columns go to the world block. When closed, the world fills
    // the full width as before. `sidebar_inner` is the inside of the
    // bordered ` side ` block — handed to `compositor.render` so it
    // can lay sidebar components out vertically inside it.
    let (map_area, sidebar_inner) = if sidebar_open && main_area.width > sidebar_width + 4 {
        let cols = Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(1)])
            .split(main_area);
        let side_area = cols[0];
        let map_area = cols[1];

        let side_block = Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.muted_color))
            .title(" side ");
        let side_inner = side_block.inner(side_area);
        f.render_widget(side_block, side_area);

        (map_area, Some(side_inner))
    } else {
        (main_area, None)
    };

    // World frame highlights when focus is on the map (= no
    // modal / sidebar component is currently active). The
    // colour rule mirrors the per-panel border in
    // `UiTheme::panel`: focused -> accent, otherwise muted.
    let map_border = if compositor.is_base_focused() {
        theme.accent
    } else {
        theme.muted_color
    };
    let map_block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(map_border))
        .title(" world ");
    let map_inner = map_block.inner(map_area);
    f.render_widget(map_block, map_area);
    if let Some(map_frame) = map_frame {
        f.render_widget(super::frame_widget::MapFrameWidget(map_frame), map_inner);

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
        // Fire the per-frame `"tick"` event on the Lua subsystem
        // against the live MapApi. This is the only per-frame
        // map-paint hook for plugins.
        lua.tick(&mut api);
    }

    // Modal panels on top of the map (bottom-up) + sidebar sections
    // laid out vertically in the side panel when it's open.
    crate::compositor::render::paint(compositor, f, map_inner, sidebar_inner, theme, ctx);

    // Empty-sidebar placeholder so toggling on with no sections shows
    // SOMETHING — otherwise the user would just see an empty box and
    // wonder if the toggle did anything.
    if let Some(side_inner) = sidebar_inner
        && compositor.sidebar_component_count() == 0
    {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "(no sections yet)",
            Style::default().fg(theme.muted_color),
        )));
        f.render_widget(placeholder, side_inner);
    }

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
