//! Compositor render orchestration — the **front side** of the
//! compositor module.
//!
//! Walks the focus stack and paints each visible component into a
//! ratatui `Frame`. Lives in a separate module from
//! [`super::Compositor`] (state + event dispatch, no ratatui) so the
//! "core" half stays ratatui-free at the import-graph level: the
//! struct definition, [`Component`] trait surface, [`Op`] vocabulary,
//! and event/poll routing are all in `mod.rs` with zero ratatui
//! references.
//!
//! Phase 2 of GitHub issue #212 (compositor split).

use ratatui::Frame;
use ratatui::layout::Rect;

use super::window::RenderWindow;
use super::{Compositor, Context, Placement, sidebar};
use crate::front::theme::UiTheme;

/// Render the compositor's stack into the given areas.
///
/// Components are routed by their [`Placement`]: `Floating`
/// (today: only the palette) renders into `map_area`; `Sidebar`
/// ones share `sidebar_area` via equal vertical split, in stack
/// order (oldest at top). When `sidebar_area` is `None` (sidebar
/// hidden), `Sidebar` components are skipped — they stay alive on
/// the stack but are invisible until the user toggles the sidebar
/// back on.
pub fn paint(
    compositor: &Compositor,
    f: &mut Frame,
    map_area: Rect,
    sidebar_area: Option<Rect>,
    theme: &UiTheme,
    ctx: &Context,
) {
    // The palette is the sole `Floating`-placement user; it
    // appears as a single instance at most (`SwitchProvider`
    // swaps in place rather than stacking) so the previous
    // bottom-up + focused-last loop collapses to a single
    // lookup — find the topmost floating component and paint
    // it. Always focused when present (the palette consumes
    // input the moment it opens).
    if let Some((idx, (_, c))) = compositor
        .stack
        .iter()
        .enumerate()
        .rev()
        .find(|(_, (_, c))| c.placement() == Placement::Floating)
    {
        let focused = idx == compositor.focused_idx;
        let mut win = RenderWindow::new(f, map_area, theme, ctx).focused(focused);
        c.render(&mut win);
    }

    if let Some(side_area) = sidebar_area {
        sidebar::render(
            f,
            side_area,
            theme,
            ctx,
            &compositor.stack,
            compositor.focused_idx,
        );
    }
}
