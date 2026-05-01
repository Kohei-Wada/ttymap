//! [`BaseLayer`] — the bottom-layer compositor component.
//!
//! Implemented as a [`Component`] that always sits at index 0 of the
//! [`Compositor`](super::Compositor) stack. Handles everything that
//! applies when no modal above it has claimed the key:
//!
//! - **Keymap fallback**: resolves `h/j/k/l/q/0/+/-/…` via [`KeyMap`]
//! - **Activation dispatch**: `:` / `/` / `i` / `?` (and any future
//!   plugin activation key) looked up in an [`Activation`] table.
//!   When the base layer sees an activation key it calls
//!   `win.open(spawn(ctx))` with the freshly-spawned component.
//! - **`gg` multi-key sequence**: tracks `pending_g` and emits
//!   `ZoomToWorld` on the second `g`.
//!
//! Tab / Shift-Tab focus cycling is **not** handled here — it's
//! intercepted by the compositor before any component sees it, so
//! BaseLayer doesn't need to know about it.
//!
//! Because it's always at the very bottom of the stack, its
//! `handle_event` only runs when the focused (possibly higher)
//! component called `win.ignore()` — exactly the old
//! "pass through to background" cases.
//!
//! The base layer renders nothing (the map comes from the render
//! thread's `MapFrame`, drawn by `App` separately from the
//! compositor).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::window::{RenderWindow, Window};
use super::{Activation, Component};
use crate::app::AppMsg;
use crate::keymap::KeyMap;
use crate::map::Action;

pub struct BaseLayer {
    keymap: KeyMap,
    /// Activation table: key event → component factory. Populated at
    /// startup from the [`Registrar`](super::Registrar) that each
    /// plugin's `register` function contributes to.
    activations: Vec<Activation>,
    /// Plugin-supplied footer hints harvested from palette entries
    /// with a non-empty `hint` (key) at startup. Rendered in the
    /// footer beside the core keymap shortcuts so the user discovers
    /// dynamically-registered key binds without hardcoding them.
    plugin_hints: Vec<(&'static str, &'static str)>,
    /// First-`g` flag of the `gg` sequence. Lives here (not in
    /// `KeyMap`) because multi-key sequencing is a base-layer
    /// concern; the keymap itself is a stateless lookup table.
    pending_g: bool,
}

impl BaseLayer {
    pub fn new(
        keymap: KeyMap,
        activations: Vec<Activation>,
        plugin_hints: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            keymap,
            activations,
            plugin_hints,
            pending_g: false,
        }
    }

    /// Find an activation matching this key (modulo Shift, which we
    /// strip to match keymap lookup semantics).
    fn activation_for(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&Activation> {
        let clean = modifiers & !KeyModifiers::SHIFT;
        self.activations
            .iter()
            .find(|a| a.code == code && a.modifiers == clean)
    }

    /// Advance the `gg` state machine and resolve via the keymap.
    /// Called unconditionally so any non-`g` keypress resets
    /// `pending_g`.
    fn resolve_keymap(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppMsg> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(AppMsg::Map(Action::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }
}

impl Component for BaseLayer {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let KeyEvent {
            code, modifiers, ..
        } = event;

        // Always advance the gg state first — vim semantics: any
        // non-`g` key (including `:`, activation keys) resets
        // `pending_g`. Tab is already filtered out by the compositor
        // above, so it never reaches here.
        let keymap_msg = self.resolve_keymap(code, modifiers);

        // Activation keys: spawn the plugin's fresh component and
        // push it on top. Some factories (Lua plugins behind a
        // `register_keybind` callback) may decline by returning
        // `None`; the key is still consumed so it doesn't fall
        // through to the keymap.
        if let Some(activation) = self.activation_for(code, modifiers) {
            if let Some(new_component) = (activation.spawn)(win.ctx()) {
                win.open(new_component);
            }
            return;
        }

        if let Some(msg) = keymap_msg {
            win.emit(msg);
        }

        // First `g` of `gg` and unrecognised keys land here. Base
        // layer has nothing below it, so we implicitly consume
        // (no `win.ignore()`).
    }

    fn render(&self, _win: &mut RenderWindow) {
        // Bottom layer draws nothing — the map is painted by `App`
        // separately from the compositor (see `App::run`).
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        // Core keymap shortcuts that always apply on the map. Plugin-
        // specific bindings (search / wiki / help / …) are appended
        // from the live registrar so the footer reflects what's
        // actually loaded — disabling or rebinding a plugin updates
        // the footer for free.
        let mut hints: Vec<(&'static str, &'static str)> =
            vec![("hjkl", "pan"), ("a/z", "zoom"), (":", "cmd")];
        hints.extend(self.plugin_hints.iter().copied());
        hints.push(("q", "quit"));
        hints
    }

    fn name(&self) -> &'static str {
        "map"
    }
}

#[cfg(test)]
mod tests {
    use super::super::Context;
    use super::super::window::WindowOps;
    use super::*;
    use crate::geo::LonLat;
    use crate::theme::ThemeId;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: Context = Context {
        center: LonLat { lon: 0.0, lat: 0.0 },
        zoom: 0.0,
        theme_id: ThemeId::Dark,
        cursor: None,
    };

    fn bg() -> BaseLayer {
        BaseLayer::new(KeyMap::default(), Vec::new(), Vec::new())
    }

    fn dispatch(bg: &mut BaseLayer, code: KeyCode) -> WindowOps {
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX);
            bg.handle_event(KeyEvent::new(code, NONE), &mut win);
        }
        ops
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut bg = bg();
        // 1st g: nothing fires, pending_g latched.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert!(ops.msgs.is_empty());
        assert!(!ops.close);
        assert!(ops.opens.is_empty());
        // 2nd g: ZoomToWorld.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert_eq!(ops.msgs, vec![AppMsg::Map(Action::ZoomToWorld)]);
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut bg = bg();
        dispatch(&mut bg, KeyCode::Char('g'));
        dispatch(&mut bg, KeyCode::Char('h')); // breaks
        // Now pending_g was reset; this g latches afresh, doesn't fire.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert!(ops.msgs.is_empty());
    }
}
