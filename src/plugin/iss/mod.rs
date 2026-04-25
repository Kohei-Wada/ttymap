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

mod opennotify;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use log::debug;

use crate::app::AppMsg;
use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Component, Context, PaletteEntry, PaletteKind, Registrar};
use crate::geo::LonLat;
use crate::map::MapApi;
use crate::plugin_api::PolledFeed;

use opennotify::{IssPosition, OpenNotifyClient};

/// Min seconds between fetches. open-notify has no published rate
/// limit; 5 s keeps load on a free public service polite while still
/// showing visible motion (~38 km between samples).
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub struct IssState {
    position: Option<IssPosition>,
    client: Arc<OpenNotifyClient>,
    feed: PolledFeed<Option<IssPosition>>,
}

impl IssState {
    pub fn new() -> Self {
        Self {
            position: None,
            client: Arc::new(OpenNotifyClient::new()),
            feed: PolledFeed::ready(REFRESH_INTERVAL),
        }
    }

    fn refresh(&mut self) {
        let client = self.client.clone();
        self.feed.refresh(move || client.current_position());
    }

    fn poll(&mut self) {
        if let Some(result) = self.feed.poll() {
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
    /// True until the first jump-to-ISS is emitted after activation.
    /// Cleared once the component nudges the map onto the station so
    /// the marker is immediately visible — user can press Enter
    /// afterwards to re-centre at any time.
    pending_initial_jump: bool,
}

impl IssComponent {
    pub fn new(state: IssHandle) -> Self {
        state.borrow_mut().refresh();
        Self {
            state,
            pending_initial_jump: true,
        }
    }
}

impl Component for IssComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        // Enter recentres the map on the cached ISS position — the
        // wiki Enter-to-jump idiom adapted for a single-target panel.
        // Pre-fetch (no cached position yet) we silently swallow Enter
        // so it doesn't leak through to the base layer mid-load.
        if event.code == KeyCode::Enter && event.modifiers == KeyModifiers::NONE {
            if let Some(pos) = self.state.borrow().position {
                win.emit(AppMsg::Jump(LonLat {
                    lat: pos.lat,
                    lon: pos.lon,
                }));
            }
            return;
        }
        // Non-modal otherwise: defer to the base layer so pan / zoom /
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

    fn poll(&mut self, win: &mut Window) {
        let pos = {
            let mut state = self.state.borrow_mut();
            state.poll();
            // Periodic re-fetch so the marker tracks the live position.
            state.refresh();
            state.position
        };
        if self.pending_initial_jump
            && let Some(p) = pos
        {
            win.emit(AppMsg::Jump(LonLat {
                lat: p.lat,
                lon: p.lon,
            }));
            self.pending_initial_jump = false;
        }
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("Enter", "fly to ISS")]
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
