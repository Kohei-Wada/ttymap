//! UI key router — pure responder-chain dispatcher.
//!
//! The router asks the focus state (via `UiState`) "who has the key?"
//! and forwards. If the focused surface returns `Effect::Pass` (or
//! there is no focused surface), the [`BackgroundResponder`] takes
//! over and resolves global keys.
//!
//! This module performs **no translation**. Per-key matching (Tab,
//! `:`, plugin activation, keymap lookup, gg) lives in
//! [`BackgroundResponder`]; per-surface matching lives in each
//! `FocusSurface` implementation.

pub mod background;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::{AppCommand, Effect, SurfaceCtx};
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::ui::UiState;

use background::BackgroundResponder;

pub struct KeyRouter {
    background: BackgroundResponder,
}

impl KeyRouter {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            background: BackgroundResponder::new(keymap),
        }
    }

    /// Expose the keymap so `app.rs` can thread it into `DispatchCtx`
    /// for the palette's key-hint renderer.
    pub fn keymap(&self) -> &KeyMap {
        self.background.keymap()
    }

    /// Walk the responder chain: focused surface → background.
    /// Returns the `AppCommand` to dispatch (if any).
    pub fn route_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ui: &mut UiState,
        center: LonLat,
    ) -> Option<AppCommand> {
        // Always advance the gg state machine first — vim semantics:
        // any key anywhere (focused surface or background) breaks `gg`.
        let keymap_fallback = self.background.resolve_keymap(code, modifiers);

        let ctx = SurfaceCtx { center };

        if let Some(effect) = ui.deliver_to_focused_surface(code, modifiers, ctx) {
            match effect {
                Effect::Consumed => return None,
                Effect::Run(cmd) => return Some(cmd),
                Effect::Pass => {} // fall through to background
            }
        }

        match self
            .background
            .handle_key(code, modifiers, ui, keymap_fallback)
        {
            Effect::Run(cmd) => Some(cmd),
            Effect::Consumed | Effect::Pass => None,
        }
    }
}
