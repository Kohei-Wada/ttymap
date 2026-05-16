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
//!
//! When a plugin stops pushing overlays (e.g. user toggles
//! `ping_simulation` off), the sink goes empty but the previously-
//! rendered MapFrame still has the lines baked in — the render
//! thread won't repaint until something invalidates the frame. We
//! catch this with a `had_overlays` flag: on the empty-sink tick
//! immediately after a non-empty one, we trigger one final
//! "clearing" redraw (with empty overlays) so the stale ghosts
//! disappear. Without this, ghosts persist until the next pan /
//! zoom forces a redraw — the [#259] symptom.

use std::time::{Duration, Instant};

use ttymap_engine::map::render::overlay::UserPolyline;

pub struct OverlayThrottle {
    sink: Vec<UserPolyline>,
    last_redraw: Instant,
    interval: Duration,
    /// True when the most recent `should_redraw → true` cycle drained
    /// non-empty overlays. Cleared by the empty-sink "clearing"
    /// redraw so the transition only fires once per cycle.
    had_overlays: bool,
}

impl OverlayThrottle {
    pub fn new(interval: Duration) -> Self {
        Self {
            sink: Vec::new(),
            last_redraw: Instant::now(),
            interval,
            had_overlays: false,
        }
    }

    /// Mutable handle to the per-frame overlay sink. `ui::draw`
    /// passes this through to the per-frame `MapApi` so plugin
    /// `on_tick` callbacks can push.
    pub fn sink_mut(&mut self) -> &mut Vec<UserPolyline> {
        &mut self.sink
    }

    /// Drain every queued overlay. Resets the buffer so the next
    /// frame starts empty.
    pub fn drain(&mut self) -> Vec<UserPolyline> {
        std::mem::take(&mut self.sink)
    }

    /// Returns `true` when:
    ///   * the sink is non-empty AND the throttle interval has
    ///     elapsed since the last overlay-driven redraw — the
    ///     normal animation path; OR
    ///   * the sink is empty BUT the previous redraw cycle had
    ///     overlays — the one-shot "clearing" redraw that wipes
    ///     stale ghosts left by a plugin that stopped pushing.
    ///
    /// On a `true` return, internal state advances so subsequent
    /// idle frames return `false` until a new push or transition.
    pub fn should_redraw(&mut self) -> bool {
        if self.sink.is_empty() {
            // Just transitioned from "had overlays" to "now empty"
            // — fire one clearing redraw so the render thread
            // produces a MapFrame without the stale overlay layer.
            if self.had_overlays {
                self.had_overlays = false;
                return true;
            }
            return false;
        }
        let now = Instant::now();
        if now.duration_since(self.last_redraw) >= self.interval {
            self.last_redraw = now;
            self.had_overlays = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly() -> UserPolyline {
        UserPolyline {
            coords: vec![],
            color: 0,
        }
    }

    #[test]
    fn empty_sink_no_history_returns_false() {
        let mut throttle = OverlayThrottle::new(Duration::ZERO);
        assert!(!throttle.should_redraw());
    }

    #[test]
    fn non_empty_sink_with_zero_interval_redraws() {
        let mut throttle = OverlayThrottle::new(Duration::ZERO);
        throttle.sink_mut().push(poly());
        assert!(throttle.should_redraw());
    }

    #[test]
    fn empty_sink_after_non_empty_fires_one_clearing_redraw() {
        // Reproduces #259: a plugin pushes an overlay, then stops.
        // The empty tick immediately after must trigger a redraw so
        // the render thread clears the stale overlay layer.
        let mut throttle = OverlayThrottle::new(Duration::ZERO);
        throttle.sink_mut().push(poly());
        assert!(throttle.should_redraw(), "non-empty + interval elapsed");
        throttle.drain();

        // Empty sink (plugin stopped pushing) — still need one
        // redraw to clear what we just sent.
        assert!(throttle.should_redraw(), "first empty tick after overlays");

        // Subsequent empty ticks must NOT keep firing — that's a
        // wasteful idle-redraw loop.
        assert!(
            !throttle.should_redraw(),
            "second empty tick must not redraw"
        );
        assert!(
            !throttle.should_redraw(),
            "third empty tick must not redraw"
        );
    }

    #[test]
    fn interval_throttles_back_to_back_pushes() {
        // Zero interval lets the first push fire, then we extend
        // the throttle so the second push has to wait. Models the
        // steady-state animation case.
        let mut throttle = OverlayThrottle::new(Duration::ZERO);
        throttle.sink_mut().push(poly());
        assert!(throttle.should_redraw(), "first push always wins");
        throttle.drain();

        // Pretend we just installed a long interval.
        throttle.interval = Duration::from_secs(60);

        throttle.sink_mut().push(poly());
        assert!(!throttle.should_redraw(), "second push within interval");
    }
}
