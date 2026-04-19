//! Async wrapper around [`WikipediaClient`].
//! Runs HTTP requests on background threads to avoid blocking the UI,
//! sharing a single client across all requests. Internal to the widget.

use std::sync::Arc;

use super::wikipedia::{WikiArticle, WikipediaClient};
use crate::shared::async_job::AsyncJob;

pub(super) struct WikiService {
    client: Arc<WikipediaClient>,
    job: AsyncJob<Vec<WikiArticle>>,
    limit: u32,
}

impl WikiService {
    pub(super) fn new(language: &str, limit: u32) -> Self {
        Self {
            client: Arc::new(WikipediaClient::new(language)),
            job: AsyncJob::new(),
            limit,
        }
    }

    /// Submit a geosearch request (coordinates → nearby articles).
    pub(super) fn geosearch(&self, lat: f64, lon: f64) {
        let client = self.client.clone();
        let limit = self.limit;
        self.job.spawn(move || client.geosearch(lat, lon, limit));
    }

    /// Poll for any completed result (non-blocking).
    pub(super) fn poll(&self) -> Option<Vec<WikiArticle>> {
        self.job.poll()
    }
}
