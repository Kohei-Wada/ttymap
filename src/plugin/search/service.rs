//! Async wrapper around forward geocoding. Plugin-internal.

use std::sync::Arc;

use crate::shared::async_job::AsyncJob;
use crate::shared::nominatim::{NominatimClient, SearchResult};

pub(super) struct SearchService {
    client: Arc<NominatimClient>,
    job: AsyncJob<Vec<SearchResult>>,
}

impl SearchService {
    pub(super) fn new(client: Arc<NominatimClient>) -> Self {
        Self {
            client,
            job: AsyncJob::new(),
        }
    }

    /// Kick off a forward search on a background thread.
    pub(super) fn search(&self, query: &str) {
        let query = query.to_string();
        let client = self.client.clone();
        self.job.spawn(move || client.search(&query));
    }

    pub(super) fn poll(&self) -> Option<Vec<SearchResult>> {
        self.job.poll()
    }
}
