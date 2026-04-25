//! Panel layout helpers for plugin-side rendering.
//!
//! Two pieces:
//!
//! - [`PanelAnchor`] — where to anchor a panel within the map area
//!   (left / right / corner / centre).
//! - [`LayoutConfig`] — a small struct plugins flatten into their
//!   own `XxxConfig` so users can adjust position and size from
//!   `config.toml` without leaving the plugin's section:
//!
//!   ```toml
//!   [wiki]
//!   language = "en"
//!   limit    = 50
//!   anchor   = "right"   # ← layout override
//!   width    = 30
//!   ```
//!
//! Plugins call [`LayoutConfig::resolve`] at render time with their
//! preferred defaults; the user's overrides win when set.

use crate::widget::Rect;
use serde::Deserialize;

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
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "top-left" | "topleft" | "tl" => Some(Self::TopLeft),
            "top-right" | "topright" | "tr" => Some(Self::TopRight),
            "bottom-left" | "bottomleft" | "bl" => Some(Self::BottomLeft),
            "bottom-right" | "bottomright" | "br" => Some(Self::BottomRight),
            "center" | "centre" => Some(Self::Center),
            _ => None,
        }
    }

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

/// User-overridable layout knobs for a panel. Designed to be
/// `#[serde(flatten)]`'d into a plugin's own config struct so the
/// fields surface at the top level of the plugin's TOML section.
///
/// Every field is optional; absent fields fall back to the plugin's
/// hardcoded defaults supplied to [`Self::resolve`].
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct LayoutConfig {
    pub anchor: Option<String>,
    pub width: Option<u16>,
    pub height: Option<u16>,
}

impl LayoutConfig {
    /// Resolve the panel rectangle by combining plugin defaults with
    /// any user-supplied overrides. Unknown anchor strings are
    /// silently ignored (default anchor wins).
    pub fn resolve(
        &self,
        outer: Rect,
        default_anchor: PanelAnchor,
        default_width: u16,
        default_height: u16,
    ) -> Rect {
        let anchor = self
            .anchor
            .as_deref()
            .and_then(PanelAnchor::from_str)
            .unwrap_or(default_anchor);
        let width = self.width.unwrap_or(default_width);
        let height = self.height.unwrap_or(default_height);
        anchor.rect(outer, width, height)
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

    #[test]
    fn config_overrides_default_anchor_when_present() {
        let cfg = LayoutConfig {
            anchor: Some("left".to_string()),
            ..Default::default()
        };
        let r = cfg.resolve(outer(), PanelAnchor::Right, 20, 30);
        assert_eq!(r.x, 1); // left edge with 1-cell margin
    }

    #[test]
    fn config_falls_back_to_default_when_unknown_anchor() {
        let cfg = LayoutConfig {
            anchor: Some("middle".to_string()),
            ..Default::default()
        };
        let r = cfg.resolve(outer(), PanelAnchor::TopLeft, 25, 5);
        assert_eq!(r.x, 1);
        assert_eq!(r.y, 1);
    }

    #[test]
    fn config_overrides_dimensions() {
        let cfg = LayoutConfig {
            width: Some(40),
            height: Some(8),
            ..Default::default()
        };
        let r = cfg.resolve(outer(), PanelAnchor::TopLeft, 20, 4);
        assert_eq!(r.width, 40);
        assert_eq!(r.height, 8);
    }
}
