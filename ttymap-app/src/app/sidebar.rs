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

    /// Apply the auto-open / auto-close invariant.
    ///
    /// Returns `true` when visibility flipped (in either direction) —
    /// the caller triggers a resize so the map canvas adjusts for
    /// the new rail state.
    ///
    /// Two symmetric transitions:
    /// - **Auto-open** on `count > prev_count` while hidden: a new
    ///   section just landed; opening it is an implicit "show me".
    /// - **Auto-close** on `prev_count > 0 → count == 0` while open:
    ///   the last card just left; an empty rail eating ~56 cols is
    ///   strictly worse than reclaiming the map area.
    ///
    /// Both fire on the *transition* — not on the steady state —
    /// so the user toggling `\` against the current count stays
    /// free. Steady-state `count == 0` with a manually-opened rail
    /// keeps the rail open until the next toggle. Mirrors VS Code's
    /// Explorer / Helix completion popup behaviour.
    pub(super) fn observe_count(&mut self, count: usize) -> bool {
        let should_open = count > self.prev_count && !self.open;
        let should_close = count == 0 && self.prev_count > 0 && self.open;
        if should_open {
            self.open = true;
        } else if should_close {
            self.open = false;
        }
        self.prev_count = count;
        should_open || should_close
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_count_auto_opens_on_first_card() {
        let mut p = SidebarPolicy::new(56);
        assert!(p.observe_count(1));
        assert!(p.open);
    }

    #[test]
    fn observe_count_auto_closes_on_last_card_removed() {
        let mut p = SidebarPolicy::new(56);
        p.observe_count(2); // open
        p.observe_count(1); // shrink, still open
        let flipped = p.observe_count(0);
        assert!(flipped, "0-transition must signal a flip");
        assert!(!p.open, "rail closes when the last card leaves");
    }

    #[test]
    fn observe_count_does_not_close_on_steady_zero() {
        // Manual `\` open against an empty stack must persist —
        // the auto-close only fires on the >0 → 0 transition,
        // not on subsequent ticks while count stays 0.
        let mut p = SidebarPolicy::new(56);
        p.toggle(); // user opens with `\`, prev_count still 0
        let flipped = p.observe_count(0);
        assert!(!flipped);
        assert!(p.open, "manual open against empty stack must stay open");
    }

    #[test]
    fn observe_count_does_not_reopen_after_user_closes_with_cards_alive() {
        // Auto-open fires on increase only. User closes manually
        // (`\`) while a card is alive — subsequent steady ticks at
        // the same count must not flip back open.
        let mut p = SidebarPolicy::new(56);
        p.observe_count(1); // auto-opens
        p.toggle(); // user closes
        let flipped = p.observe_count(1);
        assert!(!flipped);
        assert!(!p.open);
    }

    #[test]
    fn manual_toggle_against_empty_stack_then_count_arrives_does_not_flip() {
        // If the user has already toggled the rail open with `\`
        // and a card subsequently lands, observe_count should NOT
        // signal a flip (the rail is already open).
        let mut p = SidebarPolicy::new(56);
        p.toggle(); // user opens
        let flipped = p.observe_count(1);
        assert!(!flipped, "no flip when rail was already open");
        assert!(p.open);
    }
}
