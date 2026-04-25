//! ISS plugin state — owns the live position feed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::prelude::*;

use super::opennotify::{IssPosition, OpenNotifyClient};

/// Min seconds between fetches. open-notify has no published rate
/// limit; 5 s keeps load on a free public service polite while still
/// showing visible motion (~38 km between samples).
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct IssState {
    pub(super) position: Option<IssPosition>,
    client: Arc<OpenNotifyClient>,
    feed: PolledFeed<Option<IssPosition>>,
}

impl IssState {
    pub fn new() -> Self {
        Self {
            position: None,
            client: Arc::new(OpenNotifyClient::new()),
            feed: PolledFeed::ready(REFRESH_INTERVAL),
        }
    }

    pub(super) fn refresh(&mut self) {
        let client = self.client.clone();
        self.feed.refresh(move || client.current_position());
    }

    pub(super) fn poll(&mut self) {
        if let Some(result) = self.feed.poll() {
            if let Some(p) = result {
                debug!("iss: position {:.2}, {:.2}", p.lat, p.lon);
            }
            self.position = result;
        }
    }
}

impl Default for IssState {
    fn default() -> Self {
        Self::new()
    }
}

pub type IssHandle = Rc<RefCell<IssState>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_has_no_position() {
        let s = IssState::new();
        assert!(s.position.is_none());
    }
}
