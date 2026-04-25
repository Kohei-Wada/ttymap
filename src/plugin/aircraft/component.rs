//! Aircraft component — markers-only Component pushed onto the
//! compositor stack while the plugin is toggled on.

use crate::plugin_api::prelude::*;

use super::state::AircraftHandle;

/// Aircraft component — markers only, no panel. State lives behind
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
    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        // Direction-neutral glyphs only — `✈` would lock every marker
        // to the same heading regardless of true_track. Bringing back a
        // heading-aware glyph is gated on a MapApi primitive that can
        // pick from a rotated character set.
        let state = self.state.borrow();
        let fg = p.accent_color();
        let ground_fg = p.accent_alt_color();
        for a in &state.aircraft {
            let glyph = if a.on_ground { '◇' } else { '◆' };
            let color = if a.on_ground { ground_fg } else { fg };
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

    fn name(&self) -> &'static str {
        "aircraft"
    }
}
