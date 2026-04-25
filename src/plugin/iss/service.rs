//! Async wrapper around [`OpenNotifyClient`].
//! One in-flight request at a time; results land in a non-blocking
//! channel for the component's `poll`.

use std::sync::Arc;

use super::opennotify::{IssPosition, OpenNotifyClient};
use crate::shared::async_job::AsyncJob;

pub(super) struct IssService {
    client: Arc<OpenNotifyClient>,
    job: AsyncJob<Option<IssPosition>>,
}

impl IssService {
    pub(super) fn new() -> Self {
        Self {
            client: Arc::new(OpenNotifyClient::new()),
            job: AsyncJob::new(),
        }
    }

    pub(super) fn fetch(&self) {
        let client = self.client.clone();
        self.job.spawn(move || client.current_position());
    }

    pub(super) fn poll(&self) -> Option<Option<IssPosition>> {
        self.job.poll()
    }
}
