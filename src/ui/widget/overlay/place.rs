//! Top-right overlay — human-readable place name for the current map
//! center, resolved asynchronously via reverse geocoding.
//!
//! A passive widget: it has no key input, only reacts to map movement
//! (via [`PlaceWidget::on_map_moved`]) and time (via [`PlaceWidget::poll`]).
//! Draws itself through a [`MapOverlay`] impl.

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use log::debug;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::geo::LonLat;
use crate::render::frame::MapFrame;
use crate::shared::nominatim::{NominatimClient, PlaceInfo};
use crate::shared::throttle::Throttle;
use crate::ui::theme::Theme;

use super::MapOverlay;

pub struct PlaceWidget {
    name: Option<String>,
    service: PlaceService,
    throttle: Throttle,
}

impl PlaceWidget {
    pub fn new(nominatim: Arc<NominatimClient>) -> Self {
        Self {
            name: None,
            service: PlaceService::new(nominatim),
            throttle: Throttle::ready(Duration::from_secs(5)),
        }
    }

    /// Called by the app whenever the map recenters. Triggers a throttled
    /// reverse-geocode fetch.
    pub fn on_map_moved(&mut self, center: LonLat) {
        if self.throttle.check() {
            self.service.reverse(center);
        }
    }

    /// Drain any completed reverse-geocode results. Returns `true` if the
    /// displayed name changed (caller should redraw).
    pub fn poll(&mut self) -> bool {
        let Some(place) = self.service.poll() else {
            return false;
        };
        let new_name = place.map(format_name);
        if new_name != self.name {
            if let Some(ref name) = new_name {
                debug!("reverse: {}", name);
            }
            self.name = new_name;
            true
        } else {
            false
        }
    }
}

impl MapOverlay for PlaceWidget {
    fn render(&self, buf: &mut Buffer, map_area: Rect, _frame: &MapFrame, theme: &Theme) {
        let Some(ref name) = self.name else {
            return;
        };
        if map_area.width < 4 || map_area.height < 2 {
            return;
        }

        // Row 1 (just below the coords overlay).
        let width = (name.width() as u16 + 2).min(map_area.width);
        let rect = Rect::new(
            map_area.right().saturating_sub(width),
            map_area.y + 1,
            width,
            1,
        );
        Clear.render(rect, buf);
        Paragraph::new(name.clone())
            .style(Style::default().fg(theme.accent).bg(theme.bg))
            .alignment(Alignment::Right)
            .render(rect, buf);
    }
}

// ── Service: async wrapper for reverse geocoding ─────────────────────────────

struct PlaceService {
    client: Arc<NominatimClient>,
    tx: mpsc::Sender<Option<PlaceInfo>>,
    rx: mpsc::Receiver<Option<PlaceInfo>>,
}

impl PlaceService {
    fn new(client: Arc<NominatimClient>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx }
    }

    fn reverse(&self, center: LonLat) {
        let tx = self.tx.clone();
        let client = self.client.clone();
        thread::spawn(move || {
            let result = client.reverse(center.lat, center.lon);
            let _ = tx.send(result);
        });
    }

    fn poll(&self) -> Option<Option<PlaceInfo>> {
        self.rx.try_recv().ok()
    }
}

fn format_name(place: PlaceInfo) -> String {
    match (place.city, place.country) {
        (Some(city), Some(country)) => format!("{}, {}", city, country),
        (None, Some(country)) => country,
        (Some(city), None) => city,
        (None, None) => place.display_name,
    }
}
