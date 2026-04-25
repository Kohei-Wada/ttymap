//! Info plugin state — owns the reverse-geocode feed and the cached
//! place name for the top-right info bar.

use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::nominatim::{NominatimClient, PlaceInfo};
use crate::plugin_api::prelude::*;

/// Min seconds between reverse-geocode lookups. Nominatim asks
/// callers to stay under 1 req/s; 5 s is comfortably under that
/// while still reflecting pans within a few seconds.
const GEOCODE_INTERVAL: Duration = Duration::from_secs(5);

pub struct InfoState {
    pub(super) place_name: Option<String>,
    client: Arc<NominatimClient>,
    feed: PolledFeed<Option<PlaceInfo>>,
}

impl InfoState {
    pub fn new(client: Arc<NominatimClient>) -> Self {
        Self {
            place_name: None,
            client,
            feed: PolledFeed::ready(GEOCODE_INTERVAL),
        }
    }

    /// Kick off a reverse-geocode lookup for `center` if the throttle
    /// permits. Result lands later via [`Self::poll`].
    pub(super) fn refresh(&mut self, center: LonLat) {
        let client = self.client.clone();
        self.feed
            .refresh(move || client.reverse(center.lat, center.lon));
    }

    /// Drain a completed reverse-geocode result into `place_name`.
    pub(super) fn poll(&mut self) {
        if let Some(place) = self.feed.poll() {
            let new_name = place.map(format_name);
            if new_name != self.place_name {
                if let Some(ref name) = new_name {
                    debug!("info: place {}", name);
                }
                self.place_name = new_name;
            }
        }
    }
}

fn format_name(place: PlaceInfo) -> String {
    match (place.city, place.country) {
        (Some(city), Some(country)) => format!("{}, {}", city, country),
        (None, Some(country)) => country,
        (Some(city), None) => city,
        (None, None) => place.display_name,
    }
}
