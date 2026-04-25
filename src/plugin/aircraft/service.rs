//! Async wrapper around [`OpenSkyClient`].
//! Same shape as wiki/here services: a one-shot thread runs the HTTP
//! call and feeds the result back through a non-blocking channel.

use std::sync::Arc;

use super::opensky::{Aircraft, OpenSkyClient};
use crate::shared::async_job::AsyncJob;

pub(super) struct AircraftService {
    client: Arc<OpenSkyClient>,
    job: AsyncJob<Vec<Aircraft>>,
}

impl AircraftService {
    pub(super) fn new() -> Self {
        Self {
            client: Arc::new(OpenSkyClient::new()),
            job: AsyncJob::new(),
        }
    }

    /// Submit a fetch around (lat, lon).
    pub(super) fn fetch(&self, lat: f64, lon: f64, half_deg: f64) {
        let client = self.client.clone();
        self.job
            .spawn(move || client.states_around(lat, lon, half_deg));
    }

    /// Drain one completed result, if any.
    pub(super) fn poll(&self) -> Option<Vec<Aircraft>> {
        self.job.poll()
    }
}
