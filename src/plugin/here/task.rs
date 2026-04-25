//! Here task — periodic poller that surfaces the geoip-lookup
//! result as `AppMsg::Jump`. Registered in `App.tasks` so it runs
//! every tick regardless of focus state.

use crate::plugin_api::prelude::*;

use super::state::HereHandle;

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
