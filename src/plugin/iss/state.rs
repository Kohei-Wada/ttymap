//! ISS plugin state — owns the live position feed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::debug;

use crate::plugin_api::prelude::*;

use super::IssConfig;
use super::opennotify::{IssPosition, OpenNotifyClient};

pub struct IssState {
    pub(super) position: Option<IssPosition>,
    /// When the last successful position arrived. The panel renders
    /// "updated Ns ago" off this — the API itself returns a unix
    /// timestamp, but wall-clock skew makes "now - that" surprising;
    /// monotonic local Instant is what the user actually wants.
    pub(super) last_update: Option<Instant>,
    client: Arc<OpenNotifyClient>,
    feed: PolledFeed<Option<IssPosition>>,
}

impl IssState {
    pub fn new(cfg: IssConfig) -> Self {
        Self {
            position: None,
            last_update: None,
            client: Arc::new(OpenNotifyClient::new()),
            feed: PolledFeed::ready(Duration::from_secs(cfg.interval_secs)),
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
                self.last_update = Some(Instant::now());
            }
            self.position = result;
        }
    }
}

pub type IssHandle = Rc<RefCell<IssState>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_has_no_position() {
        let s = IssState::new(IssConfig::default());
        assert!(s.position.is_none());
    }
}
