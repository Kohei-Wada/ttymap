//! Sidebar layout — vertical-split sliding-window of `Sidebar`
//! components, with an outer scrollbar when more cards are open
//! than fit at once.
//!
//! Pure rendering helper extracted out of [`super::Compositor::render`]
//! so the compositor's main render path stays under 50 lines.
//! Behaviour-identical to the inline version it replaced.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

use super::window;
use super::{Component, Context, Placement};
use crate::theme::UiTheme;

/// Maximum number of sidebar cards rendered simultaneously.
/// Beyond this each section becomes too small to be useful even
/// with within-card scrolling; the sliding window + outer
/// scrollbar lets `Tab` cycle to the rest.
const MAX_VISIBLE_SIDEBAR_SECTIONS: usize = 3;

/// Render every `Placement::Sidebar` component on the stack into
/// `side_area`, capped at [`MAX_VISIBLE_SIDEBAR_SECTIONS`] visible
/// cards. The visible window slides to keep the focused card on
/// screen; an outer scrollbar surfaces the position.
pub(super) fn render(
    f: &mut Frame,
    side_area: Rect,
    theme: &UiTheme,
    ctx: &Context,
    stack: &[Box<dyn Component>],
    focused_idx: usize,
) {
    // Walk the stack once to collect sidebar refs alongside the
    // focused-in-sidebar index (counted within the sidebar slice,
    // not the global stack).
    let mut sidebar_components: Vec<&dyn Component> = Vec::new();
    let mut focused_in_sidebar: Option<usize> = None;
    for (i, c) in stack.iter().enumerate() {
        if c.placement() == Placement::Sidebar {
            if i == focused_idx {
                focused_in_sidebar = Some(sidebar_components.len());
            }
            sidebar_components.push(c.as_ref());
        }
    }

    let total = sidebar_components.len();
    if total == 0 {
        return;
    }
    let visible = total.min(MAX_VISIBLE_SIDEBAR_SECTIONS);

    // Pick the first visible index. Centre on the focused section
    // when possible; otherwise pin to the bottom (most recently
    // opened) so new sections stay visible.
    let start = if total <= visible {
        0
    } else {
        let target = focused_in_sidebar.unwrap_or(total - 1);
        let half = visible / 2;
        if target < half {
            0
        } else if target + visible - half > total {
            total - visible
        } else {
            target - half
        }
    };
    let end = start + visible;

    // Reserve the leftmost column for the sidebar-level scrollbar
    // when there's overflow. Cards render in the remaining width;
    // per-card scrollbars sit on their right border. Visually
    // distinct: outer (sidebar position) on the left, inner
    // (content scroll within a card) on the right.
    let needs_outer_scroll = total > visible;
    let cards_area = if needs_outer_scroll {
        Rect {
            x: side_area.x.saturating_add(1),
            width: side_area.width.saturating_sub(1),
            ..side_area
        }
    } else {
        side_area
    };

    let n = visible as u32;
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Ratio(1, n)).collect();
    let chunks = Layout::vertical(constraints).split(cards_area);
    for (offset, (slot, c)) in chunks
        .iter()
        .zip(sidebar_components[start..end].iter())
        .enumerate()
    {
        let global_idx = start + offset;
        let is_focused = focused_in_sidebar == Some(global_idx);
        let mut win = window::RenderWindow::new(f, *slot, theme, ctx).focused(is_focused);
        c.render(&mut win);
    }

    if needs_outer_scroll {
        render_outer_scrollbar(f, side_area, total, visible, start);
    }
}

/// Sidebar-level scrollbar: card-index-based, not row-based. Track
/// length = total cards; thumb covers the visible window. Margin
/// trim by 1 matches the per-card scrollbar pattern (rail length
/// aligns with content, doesn't bleed onto adjacent UI).
fn render_outer_scrollbar(
    f: &mut Frame,
    side_area: Rect,
    total: usize,
    visible: usize,
    start: usize,
) {
    let rail = side_area.inner(Margin {
        vertical: 1,
        horizontal: 0,
    });
    if rail.height == 0 {
        return;
    }
    // See window::RenderWindow::scrollbar for the full
    // explanation: ratatui treats `position` as [0, content-1]
    // where max means "last item at top of viewport", but we use
    // position for top-of-window with max = total - visible. Lie
    // about content so ratatui's range matches ours.
    let scaled_total = total - visible + 1;
    let mut state = ScrollbarState::new(scaled_total)
        .position(start)
        .viewport_content_length(visible);
    let bar = Scrollbar::default()
        .orientation(ScrollbarOrientation::VerticalLeft)
        .begin_symbol(None)
        .end_symbol(None);
    f.render_stateful_widget(bar, rail, &mut state);
}
