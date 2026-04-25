//! Aircraft side panel — list of visible aircraft. Lives on the
//! **left** side of the map area to stay clear of wiki's right-side
//! panel; if a future user opens both at once they get one on each
//! flank without overlap.

use crate::plugin_api::prelude::*;

use super::state::AircraftState;

pub fn render_panel(state: &AircraftState, win: &mut RenderWindow) {
    let area_outer = win.area();
    if area_outer.width < 30 || area_outer.height < 6 {
        return;
    }

    let panel_width = (area_outer.width / 4).max(28).min(area_outer.width / 3);
    let y = area_outer.y + 3;
    let panel_height = area_outer.height.saturating_sub(6);
    if panel_height < 4 {
        return;
    }

    let x = area_outer.x + 1;
    let area = Rect::new(x, y, panel_width, panel_height);
    win.clear(area);

    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);
    let selected = win.style(StyleKind::Selected);

    let visible_rows = (panel_height as usize).saturating_sub(2);
    let total = state.aircraft.len();

    let header = format!(" aircraft  ({} tracked)", total);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_rows + 1);
    lines.push(Line::from_span(Span::styled(header, body)));

    if total == 0 {
        lines.push(Line::from_span(Span::styled(" (no data yet)", muted)));
    } else {
        // Show a window of `visible_rows` items centred on `state.selected`.
        let start = state
            .selected
            .saturating_sub(visible_rows / 2)
            .min(total.saturating_sub(visible_rows.min(total)));
        let end = (start + visible_rows).min(total);

        for (i, a) in state.aircraft[start..end].iter().enumerate() {
            let idx = start + i;
            let style = if idx == state.selected {
                selected
            } else {
                body
            };
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
            lines.push(Line::from_span(Span::styled(
                format!(" {:8} {} {} {}", cs, alt, spd, ground),
                style,
            )));
        }
    }

    let paragraph = Paragraph {
        lines,
        style: body,
        framed_title: Some("aircraft".to_string()),
        ..Default::default()
    };
    win.paragraph(paragraph, area);
}
