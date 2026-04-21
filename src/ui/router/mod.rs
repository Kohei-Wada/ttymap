//! UI key router — delivers `KeyEvent`s to the focused surface and
//! handles the `Effect::Pass` fall-through + `Effect::Open` focus
//! transitions.
//!
//! Keyboard-only. Mouse input lives in [`super::mouse`] and
//! intentionally uses a *different shape* (an adapter struct that
//! owns cross-event drag state) because the two devices are not
//! symmetric:
//!
//! | | keyboard | mouse |
//! |-|-|-|
//! | model | modal/captured | position-based |
//! | state | none (pure delivery) | drag session (protocol) |
//! | routing | focus decides | hit-test decides |
//!
//! Forcing them into a shared abstraction obscures the asymmetry
//! without buying anything — every mature Rust/Go TUI surveyed
//! (helix, zellij, cursive, bottom, …) splits the two paths after
//! their common `Event` match. We do the same.
//!
//! # Key routing mechanics
//!
//! Asks [`FocusManager`] for the current [`FocusSurface`] and sends
//! the key event to it. That's the primary path. The one exception
//! is the **`Effect::Pass` fall-through**: a non-modal plugin (e.g.
//! wiki — visible *and* focused, but doesn't recognise every key)
//! returns `Pass` for keys it doesn't handle, and the router
//! redelivers those to the background responder so global keys
//! (`:` / activation keys / keymap fallback) keep working while the
//! panel has focus. Without it, pressing `i` a second time to toggle
//! wiki off would just bounce off the focused wiki surface.
//!
//! The fall-through is guarded by `was_modal` — if the focused
//! surface *is* the background, there's nowhere to fall through
//! to (and re-delivering would loop).
//!
//! `Effect::Open(id)` is handled in-line by calling
//! [`FocusManager::open`] — focus transitions don't round-trip
//! through `app_command::dispatch`, so they don't appear in the
//! returned `Option<AppCommand>`.

use crossterm::event::KeyEvent;

use crate::app_command::AppCommand;
use crate::focus::{Effect, Focus, FocusManager, FocusSurface, SurfaceCtx};

/// Route a key event to the focused surface. `ctx` is the read-only
/// app-state snapshot the surface receives (built once per event by
/// the caller from `App.theme_id` + `MapState.center` etc.). The
/// `KeyEvent` carries both the code and the modifier set in one
/// value, replacing the historical `(code, modifiers)` pair.
///
/// Free function because key routing is stateless: no drag-like
/// cross-event correlation to retain. Contrast with
/// [`super::mouse::MouseAdapter`] which is a struct precisely
/// because it holds the drag session between events.
pub fn route_key(focus: &mut FocusManager, key: KeyEvent, ctx: SurfaceCtx) -> Option<AppCommand> {
    let was_modal = !matches!(focus.current(), Focus::Background);

    let (effect, still_visible) = {
        let surface = focus.focused_surface_mut();
        let effect = surface.handle_key(key.code, key.modifiers, ctx);
        let still_visible = surface.is_visible();
        (effect, still_visible)
    };
    if !still_visible {
        focus.release_focused();
    }

    // Modal surface returned Pass → give the background a chance.
    // (Background-as-focused never reaches this branch; its own Pass
    // is the terminal "nothing happened" state.)
    let resolved = if matches!(effect, Effect::Pass) && was_modal {
        focus
            .background_mut()
            .handle_key(key.code, key.modifiers, ctx)
    } else {
        effect
    };

    match resolved {
        Effect::Run(cmd) => Some(cmd),
        Effect::Open(id) => {
            focus.open(id, ctx);
            None
        }
        Effect::Consumed | Effect::Pass => None,
    }
}
