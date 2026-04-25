//! Here plugin — "jump to current location" as a palette action.
//!
//! Headless: no component, no focus. Exists purely so the command
//! palette can offer `Jump to current location`. Selecting it kicks
//! off a background geoip lookup; when the lookup completes the
//! resolved coordinates surface as an `AppMsg::Jump` via the shared
//! [`HereTask`] polled every tick by `App`.

use std::cell::RefCell;
use std::rc::Rc;

use log::{info, warn};

use crate::app::AppMsg;
use crate::compositor::{Registrar, Task};
use crate::geo::LonLat;
use crate::shared::async_job::AsyncJob;
use crate::shared::geoip;

/// Shared state for the here subsystem. A palette activation kicks
/// off a geoip lookup; the task's `poll` drains the completed lookup
/// and emits a `Jump`.
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

    fn start_lookup(&mut self) {
        let endpoint = self.endpoint.clone();
        let timeout = self.timeout_ms;
        info!("here: starting geoip lookup");
        self.job.spawn(move || geoip::lookup(&endpoint, timeout));
    }

    fn poll_result(&mut self) -> Option<LonLat> {
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

/// Periodic poller that surfaces the Jump when the background lookup
/// completes. Registered in the app's task list.
pub struct HereTask {
    state: HereHandle,
}

impl HereTask {
    pub fn new(state: HereHandle) -> Self {
        Self { state }
    }
}

impl Task for HereTask {
    fn poll(&mut self) -> Vec<AppMsg> {
        match self.state.borrow_mut().poll_result() {
            Some(loc) => vec![AppMsg::Jump(loc)],
            None => Vec::new(),
        }
    }
}

pub fn register(endpoint: String, timeout_ms: u64, r: &mut Registrar) {
    let state: HereHandle = Rc::new(RefCell::new(HereState::new(endpoint, timeout_ms)));
    r.add_task(Box::new(HereTask::new(state.clone())));
    r.add_run("Jump to here (current location)", "", move |_| {
        state.borrow_mut().start_lookup();
        Vec::new() // Jump arrives later via HereTask::poll
    });
}
