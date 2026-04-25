//! Aircraft side panel — list of visible aircraft. Lives on the
//! **left** side by default; uses the shared `ListPanel` chrome so
//! the framing / windowing / empty state are framework-managed and
//! this file only owns row formatting.

use crate::plugin_api::prelude::*;

use super::state::AircraftState;

pub fn render_panel(state: &AircraftState, win: &mut RenderWindow) {
    let area_outer = win.area();
    if area_outer.width < 30 || area_outer.height < 6 {
        return;
    }

    let default_width = (area_outer.width / 4).max(28).min(area_outer.width / 3);
    let default_height = area_outer.height.saturating_sub(6);
    if default_height < 4 {
        return;
    }
    let area = state
        .layout
        .resolve(area_outer, PanelAnchor::Left, default_width, default_height);

    let body = win.style(StyleKind::Body);
    let selected = win.style(StyleKind::Selected);

    let rows: Vec<Line> = state
        .aircraft
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let style = if i == state.selected { selected } else { body };
            let cs = a.callsign.as_deref().unwrap_or("(no callsign)");
            let alt = a
                .altitude_m
                .map(|m| format!("{:>5}m", m as i32))
                .unwrap_or_else(|| "    -m".to_string());
            let spd = a
                .velocity_ms
                .map(|m| format!("{:>3}m/s", m as i32))
                .unwrap_or_else(|| "  -m/s".to_string());
            let ground = if a.on_ground { " ●" } else { "  " };
            Line::from_span(Span::styled(
                format!(" {:8} {} {} {}", cs, alt, spd, ground),
                style,
            ))
        })
        .collect();

    ListPanel {
        title: "aircraft".to_string(),
        subtitle: Some(format!("{} tracked", state.aircraft.len())),
        rows,
        selected: state.selected,
        empty: "(no data yet)".to_string(),
    }
    .render(area, win);
}
