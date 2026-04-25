//! Quake component — earthquake markers pushed onto the compositor
//! stack while the plugin is toggled on.

use crate::plugin_api::prelude::*;

use super::state::QuakeHandle;

/// Magnitude threshold above which a quake renders with the
/// alt-accent glyph. M5 is the rough boundary between routine
/// micro-tremors and "felt locally / occasionally damaging" events.
const NOTABLE_MAGNITUDE: f64 = 5.0;

/// Quake component — markers only, no panel. State lives behind a
/// shared handle so toggling off / on inherits the cached events
/// without an extra fetch.
pub struct QuakeComponent {
    state: QuakeHandle,
    /// True until the auto-jump to the highest-magnitude quake has
    /// fired after activation. Cleared on first successful fetch
    /// that yields any data.
    pending_initial_jump: bool,
}

impl QuakeComponent {
    pub fn new(state: QuakeHandle) -> Self {
        state.borrow_mut().refresh();
        Self {
            state,
            pending_initial_jump: true,
        }
    }
}

impl Component for QuakeComponent {
    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        let routine_fg = p.accent_color();
        let notable_fg = p.accent_alt_color();
        for q in &state.quakes {
            let (glyph, color) = if q.magnitude >= NOTABLE_MAGNITUDE {
                ('✸', notable_fg)
            } else {
                ('·', routine_fg)
            };
            p.point(
                LonLat {
                    lon: q.lon,
                    lat: q.lat,
                },
                glyph,
                color,
            );
        }
    }

    fn poll(&mut self, win: &mut Window) {
        let highest = {
            let mut state = self.state.borrow_mut();
            state.poll();
            // Periodic re-fetch so the list stays fresh while the
            // panel is open. The throttle gates actual hits.
            state.refresh();
            state.highest_magnitude().copied()
        };
        if self.pending_initial_jump
            && let Some(q) = highest
        {
            win.emit(AppMsg::Jump(LonLat {
                lat: q.lat,
                lon: q.lon,
            }));
            self.pending_initial_jump = false;
        }
    }

    fn name(&self) -> &'static str {
        "quakes"
    }
}
