//! ISS info panel — compact 3-row box at the top-left of the map
//! area. Shows the current position (in the framed title to keep the
//! body small) plus orbit constants and last-update age in two
//! content rows.
//!
//! Position is deliberately top-left so the panel doesn't compete
//! with wiki's right-side or aircraft's left-side full-height
//! panel; the 3-row footprint keeps overlap minimal even when both
//! are open.

use std::time::Instant;

use crate::plugin_api::prelude::*;

use super::state::IssState;

const PANEL_WIDTH: u16 = 30;
/// Border + 2 content rows + border.
const PANEL_HEIGHT: u16 = 4;
/// ISS orbits at ~408 km altitude with ~7.66 km/s ground-track speed.
/// These are constants of the orbit, not values from the API.
const ALTITUDE_KM: f64 = 408.0;
const VELOCITY_KMS: f64 = 7.66;

pub fn render_panel(state: &IssState, win: &mut RenderWindow) {
    let area_outer = win.area();
    if area_outer.width < PANEL_WIDTH + 2 || area_outer.height < PANEL_HEIGHT + 2 {
        return;
    }
    let area = Rect::new(
        area_outer.x + 1,
        area_outer.y + 1,
        PANEL_WIDTH,
        PANEL_HEIGHT,
    );
    win.clear(area);

    let body = win.style(StyleKind::Body);
    let muted = win.style(StyleKind::Muted);

    let title = match &state.position {
        Some(p) => format!("iss {:.2}°N, {:.2}°E", p.lat, p.lon),
        None => "iss (no position yet)".to_string(),
    };

    let lines = vec![
        Line::from_span(Span::styled(
            format!(" {:.0} km @ {:.2} km/s", ALTITUDE_KM, VELOCITY_KMS),
            body,
        )),
        Line::from_span(Span::styled(
            format!(" {}", age_text(state.last_update)),
            muted,
        )),
    ];

    let paragraph = Paragraph {
        lines,
        style: body,
        framed_title: Some(title),
        ..Default::default()
    };
    win.paragraph(paragraph, area);
}

/// "updated Ns ago" / "no data". Capped at 999s so the line never
/// exceeds its allotted width.
fn age_text(last: Option<Instant>) -> String {
    match last {
        Some(t) => {
            let secs = t.elapsed().as_secs().min(999);
            format!("updated {}s ago", secs)
        }
        None => "awaiting data".to_string(),
    }
}
