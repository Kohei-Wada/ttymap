//! Async wrapper around [`UsgsClient`].
//! One in-flight fetch at a time; results land in a non-blocking
//! channel for the component's `poll`.

use std::sync::Arc;

use super::usgs::{Quake, UsgsClient};
use crate::shared::async_job::AsyncJob;

pub(super) struct QuakeService {
    client: Arc<UsgsClient>,
    job: AsyncJob<Vec<Quake>>,
}

impl QuakeService {
    pub(super) fn new() -> Self {
        Self {
            client: Arc::new(UsgsClient::new()),
            job: AsyncJob::new(),
        }
    }

    pub(super) fn fetch(&self) {
        let client = self.client.clone();
        self.job.spawn(move || client.recent());
    }

    pub(super) fn poll(&self) -> Option<Vec<Quake>> {
        self.job.poll()
    }
}
