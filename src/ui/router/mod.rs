//! UI key router — pure delegation.
//!
//! The router asks [`FocusManager`](crate::focus::FocusManager) for
//! the current [`FocusSurface`](crate::app_command::FocusSurface) and
//! sends the key event to it. That's it. The router doesn't know
//! whether the surface is the palette, a focused plugin, or the
//! background responder — `focused_surface_mut` always returns
//! something, and the auto-release invariant lives on the focus
//! manager via `release_focused`.
//!
//! Stateless: kept as a struct rather than a free function so the
//! call shape `self.router.route_key(...)` matches the keyboard
//! adapter's call shape and leaves room for future per-router state
//! (e.g. routing telemetry).

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::{AppCommand, Effect, SurfaceCtx};
use crate::focus::FocusManager;
use crate::geo::LonLat;

/// Send the key to whichever surface `focus` currently identifies as
/// focused. After the surface returns, release focus if `is_visible`
/// flipped to false (modal surfaces only — the background is always
/// visible). Returns the `AppCommand` to dispatch (if any).
///
/// `Effect::Open(id)` is handled in-line by calling
/// [`FocusManager::open`] — focus transitions don't need to round-trip
/// through `app_command::dispatch`, so they don't appear in the
/// returned `Option<AppCommand>`.
///
/// Pure delegation otherwise: the router doesn't know whether the
/// surface is the palette, a focused plugin, or the background.
pub fn route_key(
    focus: &mut FocusManager,
    code: KeyCode,
    modifiers: KeyModifiers,
    center: LonLat,
) -> Option<AppCommand> {
    let ctx = SurfaceCtx { center };
    let (effect, still_visible) = {
        let surface = focus.focused_surface_mut();
        let effect = surface.handle_key(code, modifiers, ctx);
        let still_visible = surface.is_visible();
        (effect, still_visible)
    };
    if !still_visible {
        focus.release_focused();
    }
    match effect {
        Effect::Run(cmd) => Some(cmd),
        Effect::Open(id) => {
            focus.open(id, ctx);
            None
        }
        Effect::Consumed | Effect::Pass => None,
    }
}
