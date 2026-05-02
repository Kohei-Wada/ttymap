//! Panel layout helpers for plugin-side rendering.
//!
//! [`PanelAnchor`] is the anchor vocabulary the Lua bridge maps
//! `module.layout.anchor` strings into. Each variant places a
//! `width × height` panel inside an outer `Rect` via [`PanelAnchor::rect`].

use ratatui::layout::Rect;

/// Anchor for a panel inside the map area. Side anchors place
/// full-height stripes; corner anchors place a fixed-size box;
/// centre places a popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAnchor {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl PanelAnchor {
    /// Place a `width × height` panel inside `outer` according to
    /// this anchor. Both dimensions are clamped to fit; a 1-cell
    /// margin is left around the edges so the panel doesn't kiss
    /// the map block's border. Side anchors push the panel down by
    /// 3 rows so the top of the map (info bar territory) stays free.
    pub fn rect(self, outer: Rect, width: u16, height: u16) -> Rect {
        // Reserve a 1-cell margin from the map block border on the
        // sides chosen, plus 3 rows from the top for side stripes.
        let w = width.min(outer.width.saturating_sub(2));
        let h = height.min(outer.height.saturating_sub(2));
        let (x_off, y_off) = match self {
            Self::Left => (1, 3),
            Self::Right => (outer.width.saturating_sub(w + 1), 3),
            Self::TopLeft => (1, 1),
            Self::TopRight => (outer.width.saturating_sub(w + 1), 1),
            Self::BottomLeft => (1, outer.height.saturating_sub(h + 1)),
            Self::BottomRight => (
                outer.width.saturating_sub(w + 1),
                outer.height.saturating_sub(h + 1),
            ),
            Self::Center => (
                outer.width.saturating_sub(w) / 2,
                outer.height.saturating_sub(h) / 2,
            ),
        };
        Rect::new(outer.x + x_off, outer.y + y_off, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outer() -> Rect {
        Rect::new(0, 0, 100, 40)
    }

    #[test]
    fn anchor_right_hugs_right_edge() {
        let r = PanelAnchor::Right.rect(outer(), 20, 30);
        // 1-cell margin from right: x = 100 - 20 - 1 = 79
        assert_eq!(r.x, 79);
        assert_eq!(r.width, 20);
    }

    #[test]
    fn anchor_top_left_lands_in_corner() {
        let r = PanelAnchor::TopLeft.rect(outer(), 25, 5);
        assert_eq!(r.x, 1);
        assert_eq!(r.y, 1);
        assert_eq!(r.width, 25);
        assert_eq!(r.height, 5);
    }

    #[test]
    fn anchor_center_centres() {
        let r = PanelAnchor::Center.rect(outer(), 20, 10);
        assert_eq!(r.x, 40); // (100-20)/2
        assert_eq!(r.y, 15); // (40-10)/2
    }
}
