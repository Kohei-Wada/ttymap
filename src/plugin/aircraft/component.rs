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
        // Direction-neutral glyphs only — `✈` would lock every marker
        // to the same heading regardless of true_track. The selected
        // aircraft renders with the alt-accent so it stands out.
        let state = self.state.borrow();
        let fg = p.accent_color();
        let alt = p.accent_alt_color();
        for (i, a) in state.aircraft.iter().enumerate() {
            let selected = i == state.selected;
            let glyph = if a.on_ground { '◇' } else { '◆' };
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
