//! Aircraft plugin state — owns the live data feed and the cached
//! list of aircraft. Mutated through `refresh` (kicks off a fetch)
//! and `poll` (drains the previous fetch's result).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::prelude::*;

use super::opensky::{Aircraft, OpenSkyClient};

/// Min seconds between fetches. OpenSky anonymous quota is roughly
/// 4000 credits/day; bbox calls cost 1 credit each, so this rate
/// (~5/min) is well under the cap even left on for hours.
const REFRESH_INTERVAL: Duration = Duration::from_secs(12);

/// Half-side of the bounding box (degrees) sent to OpenSky. ±5° is
/// large enough to keep markers visible after small pans without a
/// re-fetch, small enough that the response stays compact.
const FETCH_HALF_DEG: f64 = 5.0;

pub struct AircraftState {
    pub(super) aircraft: Vec<Aircraft>,
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

    pub(super) fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        self.feed
            .refresh(move || client.states_around(center.lat, center.lon, FETCH_HALF_DEG));
    }

    pub(super) fn poll(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_empty() {
        let s = AircraftState::new();
        assert!(s.aircraft.is_empty());
    }
}
