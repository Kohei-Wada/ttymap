//! Key router — delivers `KeyEvent`s to the focused surface and
//! handles the `Effect::Pass` fall-through + `Effect::Open`
//! transitions.
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
//!
//! `KeyRouter` is stateless (no drag-like counterpart to
//! [`MouseRouter`](super::mouse::MouseRouter)) and so is a ZST. It
//! is a struct rather than a free function for symmetry with
//! `MouseRouter` — one input router per input device, owned by
//! `App`.

use crossterm::event::KeyEvent;

use crate::app_command::AppCommand;
use crate::focus::{Effect, Focus, FocusManager, FocusSurface, SurfaceCtx};

#[derive(Default)]
pub struct KeyRouter;

impl KeyRouter {
    /// Route a key event to the focused surface. `ctx` is the
    /// read-only app-state snapshot the surface receives (built once
    /// per event by the caller from `App.theme_id` + `MapState.center`
    /// etc.). The `KeyEvent` carries both the code and the modifier
    /// set in one value, replacing the historical `(code, modifiers)`
    /// pair.
    pub fn route_key(
        &self,
        focus: &mut FocusManager,
        key: KeyEvent,
        ctx: SurfaceCtx,
    ) -> Option<AppCommand> {
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
        // (Background-as-focused never reaches this branch; its own
        // Pass is the terminal "nothing happened" state.)
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
}
