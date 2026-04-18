//! Async wrapper around [`WikipediaClient`].
//! Runs HTTP requests on background threads to avoid blocking the UI,
//! sharing a single client across all requests. Internal to the widget.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use super::wikipedia::{WikiArticle, WikipediaClient};

pub(super) struct WikiService {
    client: Arc<WikipediaClient>,
    tx: mpsc::Sender<Vec<WikiArticle>>,
    rx: mpsc::Receiver<Vec<WikiArticle>>,
    limit: u32,
}

impl WikiService {
    pub(super) fn new(language: &str, limit: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            client: Arc::new(WikipediaClient::new(language)),
            tx,
            rx,
            limit,
        }
    }

    /// Submit a geosearch request (coordinates → nearby articles).
    pub(super) fn geosearch(&self, lat: f64, lon: f64) {
        let tx = self.tx.clone();
        let client = self.client.clone();
        let limit = self.limit;
        thread::spawn(move || {
            let articles = client.geosearch(lat, lon, limit);
            let _ = tx.send(articles);
        });
    }

    /// Poll for any completed result (non-blocking).
    pub(super) fn poll(&self) -> Option<Vec<WikiArticle>> {
        self.rx.try_recv().ok()
    }
}
