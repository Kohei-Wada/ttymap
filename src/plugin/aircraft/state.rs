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
    client: Arc<OpenSkyClient>,
    feed: PolledFeed<Vec<Aircraft>>,
    fetch_half_deg: f64,
}

impl AircraftState {
    pub fn new(cfg: AircraftConfig) -> Self {
        Self {
            aircraft: Vec::new(),
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
            self.aircraft = list;
        }
    }
}

pub type AircraftHandle = Rc<RefCell<AircraftState>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_empty() {
        let s = AircraftState::new(AircraftConfig::default());
        assert!(s.aircraft.is_empty());
    }
}
