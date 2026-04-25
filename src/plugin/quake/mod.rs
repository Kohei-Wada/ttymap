//! Quake plugin — recent earthquakes from the USGS public feed.
//!
//! Activated via the command palette ("Toggle quakes"). Fetches the
//! 24-hour M2.5+ summary on push and refreshes every
//! [`REFRESH_INTERVAL`]; markers disappear when the panel is popped.
//!
//! Each quake currently renders as a single cell — `·` for routine
//! tremors and `✸` (with the alt accent colour) for newsworthy
//! M5+. Magnitude / depth want graduated styling beyond a binary
//! threshold; that is on the MapApi-primitive backlog (graded color,
//! point size, label) rather than something this plugin should
//! invent locally.
//!
//! On first successful fetch the map auto-jumps to the highest-
//! magnitude quake so the user always lands somewhere meaningful —
//! matching the ISS plugin's "you toggled this on, see the thing
//! immediately" UX.

mod usgs;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use log::debug;

use crate::app::AppMsg;
use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Component, Registrar};
use crate::geo::LonLat;
use crate::map::MapApi;
use crate::plugin_api::PolledFeed;

use usgs::{Quake, UsgsClient};

/// Min seconds between fetches. The USGS feed itself updates roughly
/// every minute; 5 minutes here keeps load on a free public service
/// polite while still picking up new events promptly.
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Magnitude threshold above which a quake renders with the
/// alt-accent glyph. M5 is the rough boundary between routine
/// micro-tremors and "felt locally / occasionally damaging" events.
const NOTABLE_MAGNITUDE: f64 = 5.0;

pub struct QuakeState {
    quakes: Vec<Quake>,
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

    fn refresh(&mut self) {
        let client = self.client.clone();
        self.feed.refresh(move || client.recent());
    }

    fn poll(&mut self) {
        if let Some(list) = self.feed.poll() {
            debug!("quake: received {} events", list.len());
            self.quakes = list;
        }
    }

    fn highest_magnitude(&self) -> Option<&Quake> {
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

/// Quake component — markers only, no panel. State lives behind a
/// shared handle so toggling off / on inherits the cached events
/// without an extra fetch.
pub struct QuakeComponent {
    state: QuakeHandle,
    /// True until the auto-jump to the highest-magnitude quake has
    /// fired after activation. Cleared on first successful fetch
    /// that yields any data.
    pending_initial_jump: bool,
}

impl QuakeComponent {
    pub fn new(state: QuakeHandle) -> Self {
        state.borrow_mut().refresh();
        Self {
            state,
            pending_initial_jump: true,
        }
    }
}

impl Component for QuakeComponent {
    fn handle_event(&mut self, _event: KeyEvent, win: &mut Window) {
        // Non-modal: defer all keys to the base layer so pan / zoom /
        // quit keep working with the markers on.
        win.ignore();
    }

    fn render(&self, _win: &mut RenderWindow) {
        // No panel in v1 — the markers are the only UI.
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        let routine_fg = p.accent_color();
        let notable_fg = p.accent_alt_color();
        for q in &state.quakes {
            let (glyph, color) = if q.magnitude >= NOTABLE_MAGNITUDE {
                ('✸', notable_fg)
            } else {
                ('·', routine_fg)
            };
            p.point(
                LonLat {
                    lon: q.lon,
                    lat: q.lat,
                },
                glyph,
                color,
            );
        }
    }

    fn poll(&mut self, win: &mut Window) {
        let highest = {
            let mut state = self.state.borrow_mut();
            state.poll();
            // Periodic re-fetch so the list stays fresh while the
            // panel is open. The throttle gates actual hits.
            state.refresh();
            state.highest_magnitude().copied()
        };
        if self.pending_initial_jump
            && let Some(q) = highest
        {
            win.emit(AppMsg::Jump(LonLat {
                lat: q.lat,
                lon: q.lon,
            }));
            self.pending_initial_jump = false;
        }
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

/// Wire the quake plugin into the registrar. Palette-only activation.
pub fn register(r: &mut Registrar) {
    let state: QuakeHandle = Rc::new(RefCell::new(QuakeState::new()));
    r.add_toggle("Toggle quakes", "", move |_| {
        QuakeComponent::new(state.clone())
    });
}

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
