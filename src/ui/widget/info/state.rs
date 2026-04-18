//! Info state — the display strings that `CoordsOverlay` and
//! `ScaleBarOverlay` render. All mutations come from `app.rs` which
//! computes them from the current map state.

pub struct InfoState {
    pub(super) coords: String,
    pub(super) place: Option<String>,
    pub(super) scale_label: String,
    pub(super) scale_width: u16,
}

impl Default for InfoState {
    fn default() -> Self {
        Self::new()
    }
}

impl InfoState {
    pub fn new() -> Self {
        Self {
            coords: String::new(),
            place: None,
            scale_label: String::new(),
            scale_width: 0,
        }
    }

    pub fn set_coords(&mut self, coords: String) {
        self.coords = coords;
    }

    pub fn set_place(&mut self, place: Option<String>) {
        self.place = place;
    }

    pub fn set_scale(&mut self, label: String, width: u16) {
        self.scale_label = label;
        self.scale_width = width;
    }
}
