//! Async wrapper around the Nominatim reverse-geocode call.
//! The UI thread dispatches one request per throttle tick via `reverse`
//! and drains completions via `poll`; the HTTP call itself runs on a
//! one-shot thread per request.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::geo::LonLat;
use crate::shared::nominatim::{NominatimClient, PlaceInfo};

pub struct PlaceService {
    client: Arc<NominatimClient>,
    tx: mpsc::Sender<Option<PlaceInfo>>,
    rx: mpsc::Receiver<Option<PlaceInfo>>,
}

impl PlaceService {
    pub fn new(client: Arc<NominatimClient>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx }
    }

    pub fn reverse(&self, center: LonLat) {
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let result = client.reverse(center.lat, center.lon);
            let _ = tx.send(result);
        });
    }

    pub fn poll(&self) -> Option<Option<PlaceInfo>> {
        self.rx.try_recv().ok()
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
