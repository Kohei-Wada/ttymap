//! Per-frame overlay sink + redraw throttle.
//!
//! Lua plugins push polylines into the sink during their `on_tick`
//! callbacks; the App drains the sink into the next
//! `RenderTask::Draw`. A plugin that pushes overlays every tick would
//! otherwise trigger a full tile re-render at the main loop's ~60Hz
//! cadence — wasted work since tile data does not change between
//! frames. Throttling to ~30Hz halves render-thread CPU while
//! keeping animation visually smooth. User-event-driven redraws
//! (pan, zoom, resize, theme change) bypass the throttle and fire
//! immediately through `App::request_map_redraw`.

use std::time::{Duration, Instant};

use crate::map::render::overlay::UserPolyline;

pub(super) struct OverlayThrottle {
    sink: Vec<UserPolyline>,
    last_redraw: Instant,
    interval: Duration,
}

impl OverlayThrottle {
    pub(super) fn new(interval: Duration) -> Self {
        Self {
            sink: Vec::new(),
            last_redraw: Instant::now(),
            interval,
        }
    }

    /// Mutable handle to the per-frame overlay sink. `ui::draw`
    /// passes this through to the per-frame `MapApi` so plugin
    /// `on_tick` callbacks can push.
    pub(super) fn sink_mut(&mut self) -> &mut Vec<UserPolyline> {
        &mut self.sink
    }

    /// Drain every queued overlay. Resets the buffer so the next
    /// frame starts empty.
    pub(super) fn drain(&mut self) -> Vec<UserPolyline> {
        std::mem::take(&mut self.sink)
    }

    /// Returns `true` when the sink is non-empty AND the throttle
    /// interval has elapsed since the last overlay-driven redraw.
    /// On a `true` return, the throttle's `last_redraw` is advanced
    /// — callers don't have to track it.
    pub(super) fn should_redraw(&mut self) -> bool {
        if self.sink.is_empty() {
            return false;
        }
        let now = Instant::now();
        if now.duration_since(self.last_redraw) >= self.interval {
            self.last_redraw = now;
            return true;
        }
        false
    }
}
