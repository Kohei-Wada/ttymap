//! Here plugin — "jump to current location" as a runtime command.
//!
//! Headless: no popup, no key focus. Exists purely so the command
//! palette can offer `Jump to current location`. Activation fires a
//! background geoip lookup; when it returns the resolved coordinates
//! are surfaced through [`Plugin::pending_command`] as a
//! `Command::Jump` and the main loop recenters the map. Shares
//! `config.geoip_endpoint` / `config.geoip_timeout_ms` with the
//! `--here` startup path.

use crossterm::event::{KeyCode, KeyModifiers};
use log::{info, warn};

use crate::command::Command;
use crate::geo::LonLat;
use crate::shared::async_job::AsyncJob;
use crate::shared::geoip;

use super::{Plugin, PluginAction, PluginCtx};

pub struct HerePlugin {
    job: AsyncJob<Option<(f64, f64)>>,
    endpoint: String,
    timeout_ms: u64,
    pending: Option<LonLat>,
}

impl HerePlugin {
    pub fn new(endpoint: String, timeout_ms: u64) -> Self {
        Self {
            job: AsyncJob::new(),
            endpoint,
            timeout_ms,
            pending: None,
        }
    }
}

impl Plugin for HerePlugin {
    fn tag(&self) -> &str {
        "here"
    }

    fn description(&self) -> &str {
        "Jump to here (current location)"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        // Palette-only — no dedicated keybind in v1.
        Vec::new()
    }

    fn wants_focus(&self) -> bool {
        // Headless: fires a background job, never owns the keyboard.
        false
    }

    fn activate(&mut self, _ctx: &mut PluginCtx) {
        let endpoint = self.endpoint.clone();
        let timeout = self.timeout_ms;
        info!("here: starting geoip lookup");
        self.job.spawn(move || geoip::lookup(&endpoint, timeout));
    }

    fn visible(&self) -> bool {
        false
    }

    fn handle_key(
        &mut self,
        _code: KeyCode,
        _modifiers: KeyModifiers,
        _ctx: &mut PluginCtx,
    ) -> PluginAction {
        PluginAction::Pass
    }

    fn poll(&mut self) -> bool {
        match self.job.poll() {
            Some(Some((lat, lon))) => {
                info!("here: resolved to {}, {}", lat, lon);
                self.pending = Some(LonLat { lon, lat });
                true
            }
            Some(None) => {
                warn!("here: geoip lookup failed");
                false
            }
            None => false,
        }
    }

    fn pending_command(&mut self) -> Option<Command> {
        self.pending.take().map(Command::Jump)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_command_is_taken_once() {
        let mut p = HerePlugin::new("about:blank".into(), 100);
        p.pending = Some(LonLat { lon: 1.0, lat: 2.0 });
        assert_eq!(
            p.pending_command(),
            Some(Command::Jump(LonLat { lon: 1.0, lat: 2.0 }))
        );
        assert_eq!(p.pending_command(), None);
    }
}
