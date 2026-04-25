//! Here plugin — "jump to current location" as a palette action.
//!
//! Headless: no component, no focus. Exists purely so the command
//! palette can offer `Jump to current location`. Selecting it kicks
//! off a background geoip lookup; when the lookup completes the
//! resolved coordinates surface as an `AppMsg::Jump` via the shared
//! [`task::HereTask`] polled every tick by `App`.
//!
//! ## Layout
//!
//! - [`state`] — `HereState`: geoip lookup job
//! - [`task`] — `HereTask`: drain completed lookup → `AppMsg::Jump`

mod state;
mod task;

use std::cell::RefCell;
use std::rc::Rc;

use crate::plugin_api::prelude::*;

use state::{HereHandle, HereState};
use task::HereTask;

pub fn register(endpoint: String, timeout_ms: u64, r: &mut Registrar) {
    let state: HereHandle = Rc::new(RefCell::new(HereState::new(endpoint, timeout_ms)));
    r.add_task(Box::new(HereTask::new(state.clone())));
    r.add_run("Jump to here (current location)", "", move |_| {
        state.borrow_mut().start_lookup();
        Vec::new() // Jump arrives later via HereTask::poll
    });
}
