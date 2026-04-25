//! Aircraft plugin — live ADS-B markers from OpenSky Network.
//!
//! Activated via the command palette ("Toggle aircraft"). On push the
//! component kicks off a fetch around the current map centre; new
//! results refresh automatically every [`REFRESH_INTERVAL`]. Markers
//! disappear when the panel is popped because rendering is gated on
//! stack presence (Component::paint_on_map).
//!
//! Heading-aware glyphs, callsign labels, and on-screen detail are
//! deliberately out of scope for v1 — they are the plugin-side pain
//! points that should drive future MapApi primitives.

mod opensky;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use log::debug;

use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Component, Registrar};
use crate::geo::LonLat;
use crate::map::MapApi;
use crate::plugin_api::PolledFeed;

use opensky::{Aircraft, OpenSkyClient};

/// Min seconds between fetches. OpenSky anonymous quota is roughly
/// 4000 credits/day; bbox calls cost 1 credit each, so this rate
/// (~5/min) is well under the cap even left on for hours.
const REFRESH_INTERVAL: Duration = Duration::from_secs(12);

/// Half-side of the bounding box (degrees) sent to OpenSky. ±5° is
/// large enough to keep markers visible after small pans without a
/// re-fetch, small enough that the response stays compact.
const FETCH_HALF_DEG: f64 = 5.0;

pub struct AircraftState {
    aircraft: Vec<Aircraft>,
    client: Arc<OpenSkyClient>,
    feed: PolledFeed<Vec<Aircraft>>,
}

impl AircraftState {
    pub fn new() -> Self {
        Self {
            aircraft: Vec::new(),
            client: Arc::new(OpenSkyClient::new()),
            feed: PolledFeed::ready(REFRESH_INTERVAL),
        }
    }

    fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        self.feed
            .refresh(move || client.states_around(center.lat, center.lon, FETCH_HALF_DEG));
    }

    fn poll(&mut self) {
        if let Some(list) = self.feed.poll() {
            debug!("aircraft: received {} states", list.len());
            self.aircraft = list;
        }
    }
}

impl Default for AircraftState {
    fn default() -> Self {
        Self::new()
    }
}

pub type AircraftHandle = Rc<RefCell<AircraftState>>;

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
    fn handle_event(&mut self, _event: KeyEvent, win: &mut Window) {
        // No bound keys yet: a future iteration will add list / detail
        // panels. For v1 we are non-modal — defer all keys back to the
        // base layer so pan/zoom/quit keep working with markers on.
        win.ignore();
    }

    fn render(&self, _win: &mut RenderWindow) {
        // No panel in v1 — markers are the only UI.
    }

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

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

/// Wire the aircraft plugin into the registrar. Palette-only
/// activation for now — no dedicated key — so we don't fight the
/// existing `a → ZoomIn` binding.
pub fn register(r: &mut Registrar) {
    let state: AircraftHandle = Rc::new(RefCell::new(AircraftState::new()));
    r.add_toggle("Toggle aircraft", "", move |ctx| {
        AircraftComponent::new(state.clone(), ctx.center)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_empty() {
        let s = AircraftState::new();
        assert!(s.aircraft.is_empty());
    }
}
