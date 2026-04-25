//! ISS plugin — the International Space Station as a single moving
//! marker.
//!
//! Activated via the command palette ("Toggle ISS"). Polls Where The
//! ISS At every [`REFRESH_INTERVAL`] for the current latitude /
//! longitude and paints one glyph at that point. The marker
//! disappears when the panel is popped because rendering is gated on
//! stack presence.
//!
//! The ISS moves at roughly 7.66 km/s, so a multi-second refresh
//! produces visible motion across the map without hammering the
//! upstream API.

mod service;
mod wheretheiss;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use log::debug;

use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Component, Context, PaletteEntry, PaletteKind, Registrar};
use crate::geo::LonLat;
use crate::map::MapApi;
use crate::shared::throttle::Throttle;

use service::IssService;
use wheretheiss::IssPosition;

/// Min seconds between fetches. Where The ISS At has no documented
/// rate limit but a fair-use cap around 1 req/s; 5 s is conservative
/// while still showing visible motion (~38 km between samples).
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct IssState {
    position: Option<IssPosition>,
    service: IssService,
    throttle: Throttle,
}

impl IssState {
    pub fn new() -> Self {
        Self {
            position: None,
            service: IssService::new(),
            throttle: Throttle::ready(REFRESH_INTERVAL),
        }
    }

    fn refresh(&mut self) {
        if self.throttle.check() {
            self.service.fetch();
        }
    }

    fn poll(&mut self) {
        if let Some(result) = self.service.poll() {
            if let Some(p) = result {
                debug!("iss: position {:.2}, {:.2}", p.lat, p.lon);
            }
            self.position = result;
        }
    }
}

impl Default for IssState {
    fn default() -> Self {
        Self::new()
    }
}

pub type IssHandle = Rc<RefCell<IssState>>;

/// ISS component — markers only, no panel. Shared handle so toggling
/// off / on inherits the last fetched position (avoids a stale gap
/// while the next fetch is in flight).
pub struct IssComponent {
    state: IssHandle,
}

impl IssComponent {
    pub fn new(state: IssHandle) -> Self {
        state.borrow_mut().refresh();
        Self { state }
    }
}

impl Component for IssComponent {
    fn handle_event(&mut self, _event: KeyEvent, win: &mut Window) {
        // Non-modal: defer all keys to the base layer so pan / zoom /
        // quit keep working with the marker on.
        win.ignore();
    }

    fn render(&self, _win: &mut RenderWindow) {
        // No panel in v1 — the marker is the only UI.
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        if let Some(pos) = state.position {
            p.point(
                LonLat {
                    lon: pos.lon,
                    lat: pos.lat,
                },
                '◉',
                p.accent_alt_color(),
            );
        }
    }

    fn poll(&mut self, _win: &mut Window) {
        let mut state = self.state.borrow_mut();
        state.poll();
        // Periodic re-fetch so the marker tracks the live position.
        state.refresh();
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

/// Wire the ISS plugin into the registrar. Palette-only activation.
pub fn register(r: &mut Registrar) {
    let state: IssHandle = Rc::new(RefCell::new(IssState::new()));

    r.add_palette_entry(PaletteEntry {
        label: "Toggle ISS".to_string(),
        hint: String::new(),
        kind: PaletteKind::Toggle(Box::new(move |_ctx: &Context| -> Box<dyn Component> {
            Box::new(IssComponent::new(state.clone()))
        })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_has_no_position() {
        let s = IssState::new();
        assert!(s.position.is_none());
    }
}
