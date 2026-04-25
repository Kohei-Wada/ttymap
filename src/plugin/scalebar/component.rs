//! Scalebar component — bottom-right always-on chrome showing a
//! distance scale tied to the rendered frame's centre / zoom.

use crate::geo;
use crate::plugin_api::map_api::Anchor;
use crate::plugin_api::prelude::*;

pub struct ScalebarComponent;

impl Component for ScalebarComponent {
    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let (label, cells) = geo::scale_bar(p.center().lat, p.zoom(), p.area_width());
        if cells == 0 {
            return;
        }
        let bar = format!(
            "├{}┤ {} ",
            "─".repeat((cells as usize).saturating_sub(2)),
            label
        );
        p.text_anchored(Anchor::BottomRight, 0, &bar, p.accent_color());
    }
}
