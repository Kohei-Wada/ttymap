//! Async wrapper around the Nominatim reverse-geocode call.
//! The UI thread dispatches one request per throttle tick via `reverse`
//! and drains completions via `poll`; the HTTP call itself runs on a
//! one-shot thread per request.

use std::sync::Arc;

use crate::geo::LonLat;
use crate::plugin_api::AsyncJob;
use crate::shared::nominatim::{NominatimClient, PlaceInfo};

pub struct PlaceService {
    client: Arc<NominatimClient>,
    job: AsyncJob<Option<PlaceInfo>>,
}

impl PlaceService {
    pub fn new(client: Arc<NominatimClient>) -> Self {
        Self {
            client,
            job: AsyncJob::new(),
        }
    }

    pub fn reverse(&self, center: LonLat) {
        let client = self.client.clone();
        self.job
            .spawn(move || client.reverse(center.lat, center.lon));
    }

    pub fn poll(&self) -> Option<Option<PlaceInfo>> {
        self.job.poll()
    }
}

pub fn format_name(place: PlaceInfo) -> String {
    match (place.city, place.country) {
        (Some(city), Some(country)) => format!("{}, {}", city, country),
        (None, Some(country)) => country,
        (Some(city), None) => city,
        (None, None) => place.display_name,
    }
}
