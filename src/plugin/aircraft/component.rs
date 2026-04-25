//! Aircraft component — list panel + markers, pushed onto the
//! compositor stack while the plugin is toggled on. Up/Down move
//! the selection; Enter jumps to the highlighted aircraft.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::plugin_api::prelude::*;

use super::panel;
use super::state::AircraftHandle;

/// Aircraft component — list panel + map markers. State lives behind
/// a shared handle so toggle off / on inherits the previously
/// fetched list (avoids a fresh fetch on each open).
pub struct AircraftComponent {
    state: AircraftHandle,
}

impl AircraftComponent {
    pub fn new(state: AircraftHandle, center: LonLat) -> Self {
        state.borrow_mut().refresh(center);
        Self { state }
    }
}

impl Component for AircraftComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let mut state = self.state.borrow_mut();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        let up = (ctrl && event.code == KeyCode::Char('p')) || event.code == KeyCode::Up;
        let down = (ctrl && event.code == KeyCode::Char('n')) || event.code == KeyCode::Down;

        if state.aircraft.is_empty() {
            // Panel up but no data yet — swallow the navigation keys
            // so they don't fall through to map pan. Other keys flow
            // back to the base layer.
            if !(up || down) {
                win.ignore();
            }
            return;
        }

        if up {
            state.selected = if state.selected == 0 {
                state.aircraft.len() - 1
            } else {
                state.selected - 1
            };
            return;
        }
        if down {
            state.selected = (state.selected + 1) % state.aircraft.len();
            return;
        }
        if event.code == KeyCode::Enter {
            if let Some(a) = state.aircraft.get(state.selected) {
                win.emit(AppMsg::Jump(LonLat {
                    lat: a.lat,
                    lon: a.lon,
                }));
            }
            return;
        }
        if matches!(event.code, KeyCode::Esc | KeyCode::Backspace) {
            return;
        }

        // Non-modal: anything else flows back to the base layer so
        // pan/zoom/quit keep working with the panel up.
        win.ignore();
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(&self.state.borrow(), win);
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        // Heading-aware glyphs: 8 unicode arrows for the eight 45°
        // sectors based on true_track. Aircraft with no heading
        // (no track yet, or on the ground with no movement) fall
        // back to a diamond. Selected row in the panel renders in
        // alt-accent so the eye can match panel ↔ marker.
        let state = self.state.borrow();
        let fg = p.accent_color();
        let alt = p.accent_alt_color();
        for (i, a) in state.aircraft.iter().enumerate() {
            let selected = i == state.selected;
            let glyph = if a.on_ground {
                '◇'
            } else {
                a.heading_deg.map(heading_arrow).unwrap_or('◆')
            };
            let color = if selected { alt } else { fg };
            p.point(
                LonLat {
                    lon: a.lon,
                    lat: a.lat,
                },
                glyph,
                color,
            );
        }
    }

    fn poll(&mut self, win: &mut Window) {
        let mut state = self.state.borrow_mut();
        state.poll();
        // Periodic re-fetch so a long-open panel keeps tracking
        // without manual refresh.
        state.refresh(win.ctx().center);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("C-n/C-p", "select"), ("Enter", "jump"), (":", "close")]
    }

    fn name(&self) -> &'static str {
        "aircraft"
    }
}

/// Map a true-track in degrees (0 = north, clockwise) to one of
/// eight unicode arrows. Each arrow covers a 45° sector centred on
/// its cardinal/intercardinal direction, e.g. north spans
/// `[337.5°, 22.5°)`.
fn heading_arrow(degrees: f64) -> char {
    // Normalise to [0, 360) then offset by +22.5° so each sector
    // index lines up with `(deg + 22.5) / 45` rounding down.
    let normalised = degrees.rem_euclid(360.0);
    let sector = ((normalised + 22.5) / 45.0).floor() as usize % 8;
    // 0 = N, 1 = NE, 2 = E, ...
    const ARROWS: [char; 8] = ['↑', '↗', '→', '↘', '↓', '↙', '←', '↖'];
    ARROWS[sector]
}

#[cfg(test)]
mod tests {
    use super::heading_arrow;

    #[test]
    fn cardinal_directions() {
        assert_eq!(heading_arrow(0.0), '↑');
        assert_eq!(heading_arrow(90.0), '→');
        assert_eq!(heading_arrow(180.0), '↓');
        assert_eq!(heading_arrow(270.0), '←');
    }

    #[test]
    fn intercardinal_directions() {
        assert_eq!(heading_arrow(45.0), '↗');
        assert_eq!(heading_arrow(135.0), '↘');
        assert_eq!(heading_arrow(225.0), '↙');
        assert_eq!(heading_arrow(315.0), '↖');
    }

    #[test]
    fn sector_boundaries_round_to_neighbour() {
        // Just past the boundary picks the next sector.
        assert_eq!(heading_arrow(22.4), '↑');
        assert_eq!(heading_arrow(22.5), '↗');
        assert_eq!(heading_arrow(360.0), '↑'); // wraps
        assert_eq!(heading_arrow(-1.0), '↑'); // clamps via rem_euclid
    }
}
