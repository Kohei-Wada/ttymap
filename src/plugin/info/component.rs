//! Info component — top-right always-on chrome (centre / cursor /
//! zoom / place). Installed via `Registrar::add_overlay`; non-
//! focusable, never receives key events.

use crate::plugin_api::map_api::Anchor;
use crate::plugin_api::prelude::*;

use super::state::InfoState;

pub struct InfoComponent {
    state: InfoState,
}

impl InfoComponent {
    pub fn new(state: InfoState) -> Self {
        Self { state }
    }
}

impl Component for InfoComponent {
    fn poll(&mut self, win: &mut Window) {
        self.state.poll();
        // The reverse-geocode lookup needs a target; throttle gates
        // how often a fresh request goes out, so polling every tick
        // is cheap and naturally tracks pans/zooms.
        self.state.refresh(win.ctx().center);
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let fg = p.accent_color();

        // row 0: map centre
        let center = p.center();
        let center_line = format!(" center: {:.3}, {:.3} ", center.lat, center.lon);
        p.text_anchored(Anchor::TopRight, 0, &center_line, fg);

        // row 1: cursor lat/lon (or "unknown")
        let cursor_line = match p.cursor() {
            Some(ll) => format!(" cursor: {:.3}, {:.3} ", ll.lat, ll.lon),
            None => " cursor: unknown ".to_string(),
        };
        p.text_anchored(Anchor::TopRight, 1, &cursor_line, fg);

        // row 2: zoom
        let zoom_line = format!(" zoom: {:.1} ", p.zoom());
        p.text_anchored(Anchor::TopRight, 2, &zoom_line, fg);

        // row 3: reverse-geocoded place name
        let place = self.state.place_name.as_deref().unwrap_or("unknown");
        let place_line = format!(" place: {} ", place);
        p.text_anchored(Anchor::TopRight, 3, &place_line, fg);
    }
}
