//! Aircraft plugin state — owns the live data feed and the cached
//! list of aircraft. Mutated through `refresh` (kicks off a fetch)
//! and `poll` (drains the previous fetch's result).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::prelude::*;

use super::AircraftConfig;
use super::opensky::{Aircraft, OpenSkyClient};

pub struct AircraftState {
    pub(super) aircraft: Vec<Aircraft>,
    /// Index of the highlighted row in the panel. Up/Down move it;
    /// Enter jumps to that aircraft's location. Reset to 0 when the
    /// list shrinks below the current selection.
    pub(super) selected: usize,
    /// User-supplied panel placement override; resolved at render
    /// time. Set from `[aircraft]` config section.
    pub(super) layout: LayoutConfig,
    client: Arc<OpenSkyClient>,
    feed: PolledFeed<Vec<Aircraft>>,
    fetch_half_deg: f64,
}

impl AircraftState {
    pub fn new(cfg: AircraftConfig) -> Self {
        Self {
            aircraft: Vec::new(),
            selected: 0,
            layout: cfg.layout,
            client: Arc::new(OpenSkyClient::new()),
            feed: PolledFeed::ready(Duration::from_secs(cfg.interval_secs)),
            fetch_half_deg: cfg.fetch_half_deg,
        }
    }

    pub(super) fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        let half = self.fetch_half_deg;
        self.feed
            .refresh(move || client.states_around(center.lat, center.lon, half));
    }

    pub(super) fn poll(&mut self) {
        if let Some(list) = self.feed.poll() {
            debug!("aircraft: received {} states", list.len());
            self.apply(list);
        }
    }

    /// Replace the cached list and clamp the selection. Pulled out of
    /// [`poll`](Self::poll) so the clamping behaviour is testable
    /// without going through the threaded feed.
    fn apply(&mut self, list: Vec<Aircraft>) {
        self.aircraft = list;
        if self.selected >= self.aircraft.len() {
            self.selected = self.aircraft.len().saturating_sub(1);
        }
    }
}

pub type AircraftHandle = Rc<RefCell<AircraftState>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn ac(lat: f64, lon: f64) -> Aircraft {
        Aircraft {
            lat,
            lon,
            callsign: None,
            altitude_m: None,
            velocity_ms: None,
            heading_deg: None,
            on_ground: false,
        }
    }

    #[test]
    fn fresh_state_is_empty() {
        let s = AircraftState::new(AircraftConfig::default());
        assert!(s.aircraft.is_empty());
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn apply_replaces_list() {
        let mut s = AircraftState::new(AircraftConfig::default());
        s.apply(vec![ac(0.0, 0.0), ac(1.0, 1.0)]);
        assert_eq!(s.aircraft.len(), 2);
    }

    #[test]
    fn apply_clamps_selection_when_list_shrinks() {
        let mut s = AircraftState::new(AircraftConfig::default());
        s.apply(vec![ac(0.0, 0.0), ac(1.0, 1.0), ac(2.0, 2.0)]);
        s.selected = 2;
        s.apply(vec![ac(0.0, 0.0)]);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn apply_clamps_selection_to_zero_when_list_empties() {
        let mut s = AircraftState::new(AircraftConfig::default());
        s.apply(vec![ac(0.0, 0.0), ac(1.0, 1.0)]);
        s.selected = 1;
        s.apply(Vec::new());
        // saturating_sub(1) on len 0 keeps selected at 0 — the panel
        // renders no list, so the value is meaningless until data
        // returns, but it must stay in-bounds for the empty case.
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn apply_preserves_selection_when_still_in_range() {
        let mut s = AircraftState::new(AircraftConfig::default());
        s.apply(vec![ac(0.0, 0.0), ac(1.0, 1.0), ac(2.0, 2.0)]);
        s.selected = 1;
        s.apply(vec![ac(9.0, 9.0), ac(8.0, 8.0), ac(7.0, 7.0)]);
        assert_eq!(s.selected, 1);
    }
}
