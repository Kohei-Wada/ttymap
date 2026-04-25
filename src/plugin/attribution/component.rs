//! Attribution component — bottom-left always-on chrome showing the
//! tile provider's attribution text. Static: the text is fixed at
//! plugin registration.

use crate::plugin_api::map_api::Anchor;
use crate::plugin_api::prelude::*;

pub struct AttributionComponent {
    text: String,
}

impl AttributionComponent {
    pub fn new(text: String) -> Self {
        Self { text }
    }
}

impl Component for AttributionComponent {
    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        if self.text.is_empty() {
            return;
        }
        // Muted — attribution is required but shouldn't compete with
        // live chrome.
        p.text_anchored(Anchor::BottomLeft, 0, &self.text, p.muted_color());
    }
}
