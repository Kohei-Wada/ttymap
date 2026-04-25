//! Quake plugin state — owns the live earthquake feed and the
//! cached list.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use log::debug;

use crate::plugin_api::prelude::*;

use super::usgs::{Quake, UsgsClient};

/// Min seconds between fetches. The USGS feed itself updates roughly
/// every minute; 5 minutes here keeps load on a free public service
/// polite while still picking up new events promptly.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

pub struct QuakeState {
    pub(super) quakes: Vec<Quake>,
    client: Arc<UsgsClient>,
    feed: PolledFeed<Vec<Quake>>,
}

impl QuakeState {
    pub fn new() -> Self {
        Self {
            quakes: Vec::new(),
            client: Arc::new(UsgsClient::new()),
            feed: PolledFeed::ready(REFRESH_INTERVAL),
        }
    }

    pub(super) fn refresh(&mut self) {
        let client = self.client.clone();
        self.feed.refresh(move || client.recent());
    }

    pub(super) fn poll(&mut self) {
        if let Some(list) = self.feed.poll() {
            debug!("quake: received {} events", list.len());
            self.quakes = list;
        }
    }

    pub(super) fn highest_magnitude(&self) -> Option<&Quake> {
        self.quakes.iter().max_by(|a, b| {
            a.magnitude
                .partial_cmp(&b.magnitude)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

impl Default for QuakeState {
    fn default() -> Self {
        Self::new()
    }
}

pub type QuakeHandle = Rc<RefCell<QuakeState>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_empty() {
        let s = QuakeState::new();
        assert!(s.quakes.is_empty());
        assert!(s.highest_magnitude().is_none());
    }

    #[test]
    fn highest_magnitude_picks_max() {
        let mut s = QuakeState::new();
        s.quakes = vec![
            Quake {
                lat: 0.0,
                lon: 0.0,
                magnitude: 3.0,
            },
            Quake {
                lat: 1.0,
                lon: 1.0,
                magnitude: 6.5,
            },
            Quake {
                lat: 2.0,
                lon: 2.0,
                magnitude: 4.7,
            },
        ];
        let top = s.highest_magnitude().expect("should pick");
        assert!((top.magnitude - 6.5).abs() < 1e-9);
    }
}
