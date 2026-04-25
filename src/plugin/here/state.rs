//! Here plugin state — owns the geoip lookup job. A palette
//! activation triggers `start_lookup`; [`super::task::HereTask`]
//! drains the completed result every tick.

use std::cell::RefCell;
use std::rc::Rc;

use log::{info, warn};

use crate::plugin_api::prelude::*;
use crate::shared::async_job::AsyncJob;
use crate::shared::geoip;

pub struct HereState {
    job: AsyncJob<Option<(f64, f64)>>,
    endpoint: String,
    timeout_ms: u64,
}

impl HereState {
    pub fn new(endpoint: String, timeout_ms: u64) -> Self {
        Self {
            job: AsyncJob::new(),
            endpoint,
            timeout_ms,
        }
    }

    pub(super) fn start_lookup(&mut self) {
        let endpoint = self.endpoint.clone();
        let timeout = self.timeout_ms;
        info!("here: starting geoip lookup");
        self.job.spawn(move || geoip::lookup(&endpoint, timeout));
    }

    pub(super) fn poll_result(&mut self) -> Option<LonLat> {
        match self.job.poll() {
            Some(Some((lat, lon))) => {
                info!("here: resolved to {}, {}", lat, lon);
                Some(LonLat { lon, lat })
            }
            Some(None) => {
                warn!("here: geoip lookup failed");
                None
            }
            None => None,
        }
    }
}

pub type HereHandle = Rc<RefCell<HereState>>;
