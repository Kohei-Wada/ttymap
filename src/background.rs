//! Bottom-layer compositor component — the host's default key handler
//! when no modal is on top.
//!
//! Implemented as a [`Component`] that always sits at index 0 of the
//! [`Compositor`](crate::compositor::Compositor) stack. Plays the role
//! today's `BackgroundResponder` plays under `FocusManager`, collapsed
//! into the same primitive every other focus surface uses.
//!
//! Responsibilities:
//! - **Keymap fallback**: resolves `h/j/k/l/q/0/+/-/…` via [`KeyMap`]
//! - **Activation dispatch**: `:` / `/` / `i` / `?` (and any future
//!   plugin activation key) looked up in an [`Activation`] table,
//!   each entry a `KeyEvent → SpawnComponent`. When the bottom layer
//!   sees an activation key it returns [`EventResult::Push`] with
//!   the freshly-spawned component.
//! - **Tab cycle**: emits `AppMsg::CycleFocus(forward)`; the
//!   compositor rotates on dispatch.
//! - **`gg` multi-key sequence**: tracks `pending_g` and emits
//!   `ZoomToWorld` on the second `g`.
//!
//! Because the bottom layer is always at the very bottom of the
//! stack, its `handle_event` only runs when every modal above it has
//! returned [`EventResult::Ignored`] — exactly the cases the old
//! `Effect::Pass` → background-redelivery branch handled.
//!
//! The bottom layer renders nothing (the map comes from the render
//! thread's `MapFrame`, drawn by `App` separately from the
//! compositor).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::AppMsg;
use crate::compositor::{Activation, Component, Context, EventResult};
use crate::keymap::KeyMap;
use crate::map::Action;

pub struct BackgroundResponder {
    keymap: KeyMap,
    /// Activation table: key event → component factory. Populated at
    /// startup from the [`Registrar`](crate::compositor::Registrar)
    /// that each plugin's `register` function contributes to.
    activations: Vec<Activation>,
    /// First-`g` flag of the `gg` sequence. Lives here (not in
    /// `KeyMap`) because multi-key sequencing is a responder concern;
    /// the keymap itself is a stateless lookup table.
    pending_g: bool,
}

impl BackgroundResponder {
    pub fn new(keymap: KeyMap, activations: Vec<Activation>) -> Self {
        Self {
            keymap,
            activations,
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

impl Component for BackgroundResponder {
    fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> EventResult {
        let KeyEvent { code, modifiers, .. } = event;

        // Always advance the gg state first — vim semantics: any
        // non-`g` key (including focus-transition triggers like Tab,
        // `:`, activation keys) resets `pending_g`.
        let keymap_msg = self.resolve_keymap(code, modifiers);

        // Tab / Shift-Tab → cycle compositor stack.
        let forward = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward || backward {
            return EventResult::Consumed(vec![AppMsg::CycleFocus(forward)]);
        }

        // Activation keys: spawn the plugin's fresh component and
        // push it on top.
        if let Some(activation) = self.activation_for(code, modifiers) {
            let new_component = (activation.spawn)(ctx);
            return EventResult::Push(new_component, Vec::new());
        }

        if let Some(msg) = keymap_msg {
            return EventResult::Consumed(vec![msg]);
        }

        // First `g` of `gg` and unrecognised keys land here. Bottom
        // layer has nothing below it, so we consume rather than
        // Ignore — there is no lower layer to fall through to.
        EventResult::Consumed(Vec::new())
    }

    fn render(&self, _f: &mut ratatui::Frame, _area: ratatui::layout::Rect, _theme: &crate::theme::UiTheme) {
        // Bottom layer draws nothing — the map is painted by `App`
        // separately from the compositor (see `App::run`).
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("hjkl", "pan"),
            ("a/z", "zoom"),
            (":", "cmd"),
            ("/", "search"),
            ("i", "wiki"),
            ("?", "help"),
            ("q", "quit"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_palette::ThemeId;
    use crate::geo::LonLat;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: Context = Context {
        center: LonLat { lon: 0.0, lat: 0.0 },
        theme_id: ThemeId::Dark,
    };

    fn bg() -> BackgroundResponder {
        BackgroundResponder::new(KeyMap::default(), Vec::new())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, NONE)
    }

    fn assert_consumed_msg(effect: EventResult, expected: AppMsg) {
        match effect {
            EventResult::Consumed(msgs) => assert_eq!(msgs, vec![expected]),
            _ => panic!("expected Consumed, got something else"),
        }
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut bg = bg();
        // 1st g: nothing fires, pending_g latched.
        match bg.handle_event(key(KeyCode::Char('g')), &CTX) {
            EventResult::Consumed(msgs) => assert!(msgs.is_empty()),
            _ => panic!("expected Consumed(empty)"),
        }
        // 2nd g: ZoomToWorld.
        assert_consumed_msg(
            bg.handle_event(key(KeyCode::Char('g')), &CTX),
            AppMsg::Map(Action::ZoomToWorld),
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut bg = bg();
        bg.handle_event(key(KeyCode::Char('g')), &CTX);
        bg.handle_event(key(KeyCode::Char('h')), &CTX); // breaks
        // Now pending_g was reset; this g latches afresh, doesn't fire.
        match bg.handle_event(key(KeyCode::Char('g')), &CTX) {
            EventResult::Consumed(msgs) => assert!(msgs.is_empty()),
            _ => panic!("expected Consumed(empty)"),
        }
    }
}
