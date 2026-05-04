//! Left-sidebar visibility / width / auto-open invariant.
//!
//! Three fields used to live flat on `App`. Grouping them keeps the
//! auto-open rule local — observing a count change and flipping
//! `open` is one method call instead of three field assignments
//! scattered through `poll_compositor`.

pub(super) struct SidebarPolicy {
    /// Whether the sidebar is currently visible.
    pub(super) open: bool,
    /// Width in terminal cells when visible. Sourced from
    /// `ttymap.opt.runtime.sidebar_width` at startup.
    pub(super) width: u16,
    /// Component count observed at the previous poll. Drives the
    /// auto-open-on-increase rule.
    prev_count: usize,
}

impl SidebarPolicy {
    pub(super) fn new(width: u16) -> Self {
        Self {
            open: false,
            width,
            prev_count: 0,
        }
    }

    /// Flip visibility. The caller is responsible for the follow-up
    /// resize that adjusts the map canvas — the policy doesn't know
    /// about the render thread.
    pub(super) fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Apply the auto-open-on-increase invariant.
    ///
    /// Returns `true` when a *new* section has just landed and the
    /// sidebar was hidden — the caller should trigger a resize so
    /// the map canvas shrinks for the now-visible rail.
    ///
    /// Triggering off "any sidebar component exists" instead would
    /// force the sidebar back open on every poll tick after the user
    /// closed it via `\` — they could never hide the sidebar while a
    /// wiki / aircraft panel was alive on the stack. Tracking the
    /// previous count lets the user toggle freely; opening a *new*
    /// section still asserts itself, since asking for new content is
    /// an implicit "show me this".
    pub(super) fn observe_count(&mut self, count: usize) -> bool {
        let should_open = count > self.prev_count && !self.open;
        if should_open {
            self.open = true;
        }
        self.prev_count = count;
        should_open
    }

    /// Terminal-cell width of the visible map area, accounting for
    /// the sidebar when it's open. Very narrow terminals skip the
    /// sidebar entirely so the map keeps a usable width — mirrors
    /// the `ui::draw` layout fallback.
    pub(super) fn effective_map_cols(&self, full_cols: u16) -> u16 {
        if self.open && full_cols > self.width + 4 {
            full_cols.saturating_sub(self.width)
        } else {
            full_cols
        }
    }
}
