//! One-shot "auto-recentre when the first data arrives" helper.
//!
//! Several plugins (iss, quake) auto-jump the map onto their feature
//! the first time a fetch yields a target — so the user lands
//! somewhere meaningful right after toggling the plugin on, without
//! having to hunt for the marker. The bookkeeping is the same shape
//! every time: a single `bool`, a check during `poll`, an emit + flag
//! clear when the target appears.
//!
//! [`InitialJump`] folds that shape into one type. A plugin holds it
//! alongside its state and calls [`try_fire`](Self::try_fire) on each
//! `poll`. Once a target shows up, the helper emits `AppMsg::Jump`
//! through the supplied [`Window`] and stays cleared for the rest of
//! the component's lifetime.

use crate::app::AppMsg;
use crate::compositor::window::Window;
use crate::geo::LonLat;

/// One-shot auto-jump primitive. Fires `AppMsg::Jump` through `win`
/// the first time [`try_fire`](Self::try_fire) is called with
/// `Some(target)`, then stays inert.
pub struct InitialJump {
    pending: bool,
}

impl InitialJump {
    /// Build a primed instance — the next [`try_fire`] with a
    /// `Some(target)` will emit. Use this in `Component::new` for
    /// plugins that should auto-jump on first fetch.
    pub fn new() -> Self {
        Self { pending: true }
    }

    /// If still primed and `target` is `Some`, emit `AppMsg::Jump` and
    /// disarm. Calling with `None` while still primed is a no-op (the
    /// helper waits for real data). Calling after firing — primed or
    /// not — is also a no-op.
    pub fn try_fire(&mut self, target: Option<LonLat>, win: &mut Window) {
        if self.pending
            && let Some(ll) = target
        {
            win.emit(AppMsg::Jump(ll));
            self.pending = false;
        }
    }
}

impl Default for InitialJump {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Context;
    use crate::compositor::window::WindowOps;
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::default(),
            cursor: None,
        }
    }

    fn jumps(ops: &WindowOps) -> Vec<LonLat> {
        ops.msgs
            .iter()
            .filter_map(|m| match m {
                AppMsg::Jump(ll) => Some(*ll),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fires_once_when_target_arrives() {
        let mut ij = InitialJump::new();
        let target = LonLat {
            lat: 35.68,
            lon: 139.76,
        };
        let ctx = ctx();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &ctx);
            ij.try_fire(Some(target), &mut win);
        }
        assert_eq!(jumps(&ops), vec![target]);
    }

    #[test]
    fn no_emit_while_target_is_none() {
        let mut ij = InitialJump::new();
        let ctx = ctx();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &ctx);
            ij.try_fire(None, &mut win);
        }
        assert!(jumps(&ops).is_empty());
    }

    #[test]
    fn second_call_is_a_noop() {
        let mut ij = InitialJump::new();
        let target = LonLat { lat: 1.0, lon: 2.0 };
        let ctx = ctx();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &ctx);
            ij.try_fire(Some(target), &mut win);
            ij.try_fire(Some(LonLat { lat: 9.9, lon: 9.9 }), &mut win);
        }
        // Only the first target was emitted; the second call is inert.
        assert_eq!(jumps(&ops), vec![target]);
    }

    #[test]
    fn none_then_some_still_fires() {
        // Common flow: poll arrives empty for a few ticks, then real
        // data comes in. The helper must wait, not waste its shot.
        let mut ij = InitialJump::new();
        let target = LonLat { lat: 5.0, lon: 6.0 };
        let ctx = ctx();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &ctx);
            ij.try_fire(None, &mut win);
            ij.try_fire(None, &mut win);
            ij.try_fire(Some(target), &mut win);
        }
        assert_eq!(jumps(&ops), vec![target]);
    }
}
