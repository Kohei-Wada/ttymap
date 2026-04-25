//! ISS component — moving marker + compact info panel pushed onto
//! the compositor stack while the plugin is toggled on.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::plugin_api::prelude::*;

use super::panel;
use super::state::IssHandle;

/// ISS component — marker on the map + a compact info panel. Shared
/// handle so toggling off / on inherits the last fetched position
/// (avoids a stale gap while the next fetch is in flight).
pub struct IssComponent {
    state: IssHandle,
    /// True until the first jump-to-ISS is emitted after activation.
    /// Cleared once the component nudges the map onto the station so
    /// the marker is immediately visible — user can press Enter
    /// afterwards to re-centre at any time.
    pending_initial_jump: bool,
}

impl IssComponent {
    pub fn new(state: IssHandle) -> Self {
        state.borrow_mut().refresh();
        Self {
            state,
            pending_initial_jump: true,
        }
    }
}

impl Component for IssComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        // Enter recentres the map on the cached ISS position — the
        // wiki Enter-to-jump idiom adapted for a single-target panel.
        // Pre-fetch (no cached position yet) we silently swallow Enter
        // so it doesn't leak through to the base layer mid-load.
        if event.code == KeyCode::Enter && event.modifiers == KeyModifiers::NONE {
            if let Some(pos) = self.state.borrow().position {
                win.emit(AppMsg::Jump(LonLat {
                    lat: pos.lat,
                    lon: pos.lon,
                }));
            }
            return;
        }
        // Non-modal otherwise: defer to the base layer so pan / zoom /
        // quit keep working with the marker on.
        win.ignore();
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(&self.state.borrow(), win);
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        if let Some(pos) = state.position {
            let ll = LonLat {
                lon: pos.lon,
                lat: pos.lat,
            };
            let color = p.accent_alt_color();
            p.point(ll, '◉', color);
            p.label(ll, " ISS", color);
        }
    }

    fn poll(&mut self, win: &mut Window) {
        let pos = {
            let mut state = self.state.borrow_mut();
            state.poll();
            // Periodic re-fetch so the marker tracks the live position.
            state.refresh();
            state.position
        };
        if self.pending_initial_jump
            && let Some(p) = pos
        {
            win.emit(AppMsg::Jump(LonLat {
                lat: p.lat,
                lon: p.lon,
            }));
            self.pending_initial_jump = false;
        }
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("Enter", "fly to ISS")]
    }

    fn name(&self) -> &'static str {
        "iss"
    }
}
