//! Wiki UI — side panel (list/detail) plus a marker-points adapter
//! for the shared [`MarkersOverlay`].
//!
//! State lives in [`WikiState`] on `UiState`. The panel renderer and
//! the `marker_points` adapter both borrow this state, so they always
//! agree on what is selected.

pub mod panel;
pub mod state;

pub use panel::render_panel;
pub use state::{WikiAction, WikiState};

use crate::ui::theme::Theme;
use crate::ui::widget::overlay::MarkerPoint;

/// Adapt wiki state into a `Vec<MarkerPoint>` for [`MarkersOverlay`].
/// Returns an empty vec when the panel is inactive so nothing is drawn.
pub fn marker_points(state: &WikiState, theme: &Theme) -> Vec<MarkerPoint> {
    if !state.is_active() {
        return Vec::new();
    }
    state
        .articles
        .iter()
        .enumerate()
        .map(|(i, a)| MarkerPoint {
            lon: a.lon,
            lat: a.lat,
            glyph: '●',
            fg: if i == state.selected {
                theme.accent_alt
            } else {
                theme.accent
            },
        })
        .collect()
}
