//! Geocoding service — async wrapper around NominatimClient.
//! Runs HTTP requests on background threads to avoid blocking the UI.
//! Shares a single NominatimClient across all requests.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::geo::LonLat;
use crate::nominatim::{NominatimClient, PlaceInfo, SearchResult};

/// Async geocoding results.
pub enum GeoResponse {
    Search(Vec<SearchResult>),
    Completion(Vec<SearchResult>),
    Reverse(Option<PlaceInfo>),
}

pub struct Geocoder {
    client: Arc<NominatimClient>,
    tx: mpsc::Sender<GeoResponse>,
    rx: mpsc::Receiver<GeoResponse>,
}

impl Default for Geocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Geocoder {
    pub fn new() -> Self {
        let client = NominatimClient::new().expect("failed to create HTTP client");
        let (tx, rx) = mpsc::channel();
        Self {
            client: Arc::new(client),
            tx,
            rx,
        }
    }

    /// Submit a forward search query (place name → coordinates).
    pub fn search(&self, query: &str) {
        let query = query.to_string();
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let results = client.search(&query);
            let _ = tx.send(GeoResponse::Search(results));
        });
    }

    /// Submit a completion query (same as search, tagged differently).
    pub fn complete(&self, query: &str) {
        let query = query.to_string();
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let results = client.search(&query);
            let _ = tx.send(GeoResponse::Completion(results));
        });
    }

    /// Submit a reverse geocoding request (coordinates → place info).
    pub fn reverse(&self, location: LonLat) {
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let result = client.reverse(location.lat, location.lon);
            let _ = tx.send(GeoResponse::Reverse(result));
        });
    }

    /// Poll for any completed result (non-blocking).
    pub fn poll(&self) -> Option<GeoResponse> {
        self.rx.try_recv().ok()
    }
}
