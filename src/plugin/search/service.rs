//! Async wrapper around forward geocoding. Plugin-internal.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::shared::nominatim::{NominatimClient, SearchResult};

pub(super) struct SearchService {
    client: Arc<NominatimClient>,
    tx: mpsc::Sender<Vec<SearchResult>>,
    rx: mpsc::Receiver<Vec<SearchResult>>,
}

impl SearchService {
    pub(super) fn new(client: Arc<NominatimClient>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx }
    }

    /// Kick off a forward search on a background thread.
    pub(super) fn search(&self, query: &str) {
        let query = query.to_string();
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let results = client.search(&query);
            let _ = tx.send(results);
        });
    }

    pub(super) fn poll(&self) -> Option<Vec<SearchResult>> {
        self.rx.try_recv().ok()
    }
}
