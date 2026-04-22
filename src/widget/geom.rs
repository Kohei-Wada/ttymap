//! Geometry primitives — `Rect` (mirror of `ratatui::layout::Rect`),
//! `Size` (constraint replacement), and `split_rows` helper.

use ratatui::layout::{Constraint, Direction, Layout, Rect as RRect};

/// Absolute-coordinate rectangle. Mirror of `ratatui::layout::Rect`.
///
/// Plugins construct these for sub-areas inside the area they were
/// allocated (`win.area()`). The compositor clamps to the allocated
/// region before drawing, so arithmetic overflow or out-of-bounds
/// coordinates cannot escape the component's slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub const fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }
}

impl From<Rect> for RRect {
    fn from(r: Rect) -> Self {
        RRect::new(r.x, r.y, r.width, r.height)
    }
}

impl From<RRect> for Rect {
    fn from(r: RRect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// Row/column sizing directive for `split_rows`. Mirror of the
/// subset of `ratatui::layout::Constraint` that plugins actually
/// use (`Length` and `Min`); extend if new variants are needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// Exact cell count.
    Fixed(u16),
    /// At-least cell count; remaining space flows here.
    Min(u16),
}

impl From<Size> for Constraint {
    fn from(s: Size) -> Self {
        match s {
            Size::Fixed(n) => Constraint::Length(n),
            Size::Min(n) => Constraint::Min(n),
        }
    }
}

/// Split `area` vertically into rectangles sized by `sizes`. Thin
/// wrapper around `ratatui::layout::Layout` so plugin code never
/// imports ratatui.
pub fn split_rows(area: Rect, sizes: &[Size]) -> Vec<Rect> {
    let constraints: Vec<Constraint> = sizes.iter().copied().map(Into::into).collect();
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area.into());
    parts.iter().copied().map(Into::into).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_roundtrip() {
        let r = Rect::new(3, 5, 20, 10);
        let rr: RRect = r.into();
        let back: Rect = rr.into();
        assert_eq!(r, back);
    }

    #[test]
    fn rect_right_bottom() {
        let r = Rect::new(10, 20, 5, 7);
        assert_eq!(r.right(), 15);
        assert_eq!(r.bottom(), 27);
    }

    #[test]
    fn size_to_constraint() {
        assert!(matches!(
            Constraint::from(Size::Fixed(3)),
            Constraint::Length(3)
        ));
        assert!(matches!(Constraint::from(Size::Min(5)), Constraint::Min(5)));
    }

    #[test]
    fn split_rows_sums_to_area_height() {
        let area = Rect::new(0, 0, 20, 10);
        let parts = split_rows(area, &[Size::Fixed(1), Size::Fixed(1), Size::Min(1)]);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].height, 1);
        assert_eq!(parts[1].height, 1);
        assert_eq!(parts[2].height, 8);
        let total: u16 = parts.iter().map(|r| r.height).sum();
        assert_eq!(total, area.height);
    }
}
