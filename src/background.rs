//! Background responder — the host's "default" key handler when no
//! palette / plugin has focus.
//!
//! Conceptually a peer of [`CommandPalette`](crate::plugin::palette::CommandPalette)
//! and the plugin registry: each is a "thing that handles keys", just
//! at a different focus state. Owned by [`FocusManager`](crate::focus::FocusManager),
//! returned from `focused_surface_mut` whenever no modal surface holds
//! focus — so the router never sees a `None` and never special-cases
//! the background.
//!
//! Owns the three global key behaviours:
//! - Tab / Shift-Tab → cycle focus across visible plugins
//! - plugin activation keys (`:`, `/`, `i`, `?`, …) → activate
//!   (palette is a builtin plugin, so `:` flows through this path
//!   like every other activation key)
//! - keymap fallback (h/j/k/l/q/0/+/-/…) → map action
//!
//! Plus the `gg` multi-key sequence state machine.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::AppCommand;
use crate::focus::{Effect, FocusSurface, SurfaceCtx};
use crate::keymap::{KeyBinding, KeyMap};
use crate::map::Action;

pub struct BackgroundResponder {
    keymap: KeyMap,
    /// `(KeyBinding, plugin_tag)` pairs harvested from every plugin's
    /// `activation_keys()` at startup. Owned here so the responder
    /// can look up activations without borrowing the plugin registry
    /// at delivery time.
    activations: Vec<(KeyBinding, String)>,
    /// First-`g` flag of the `gg` sequence. Lives here (not in
    /// `KeyMap`) because multi-key sequencing is a responder concern;
    /// the keymap itself is a stateless lookup table.
    pending_g: bool,
}

impl BackgroundResponder {
    pub fn new(keymap: KeyMap, activations: Vec<(KeyBinding, String)>) -> Self {
        Self {
            keymap,
            activations,
            pending_g: false,
        }
    }

    fn activation_tag(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&str> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.activations
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, tag)| tag.as_str())
    }

    /// Advance the `gg` state machine and resolve via the keymap.
    /// Used internally by `handle_key`; called unconditionally so any
    /// non-`g` keypress resets `pending_g`.
    fn resolve_keymap(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppCommand> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(AppCommand::Map(Action::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }
}

/// `BackgroundResponder` is a `FocusSurface` (`is_visible` = true,
/// always available) so the router can deliver keys to it through the
/// same `&mut dyn FocusSurface` channel as palette / plugin. It needs
/// `widgets` from `SurfaceCtx` to resolve plugin activation keys.
impl FocusSurface for BackgroundResponder {
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers, _ctx: SurfaceCtx) -> Effect {
        // Always advance the gg state first — vim semantics: any
        // non-`g` key (including focus-transition triggers like Tab,
        // `:`, activation keys) resets `pending_g`.
        let keymap_cmd = self.resolve_keymap(code, modifiers);

        let forward = code == KeyCode::Tab && modifiers == KeyModifiers::NONE;
        let backward = code == KeyCode::BackTab
            || (code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT));
        if forward || backward {
            return Effect::Run(AppCommand::CycleFocus(forward));
        }

        // `:` is no longer a special-case here — palette is a builtin
        // plugin with `activation_keys() = [":"]`, so the activation
        // table below catches it through the same lookup as `/`, `i`,
        // `?`. Keeps the surface count to one.
        if let Some(tag) = self.activation_tag(code, modifiers) {
            return Effect::Open(tag.to_string().into());
        }

        if let Some(cmd) = keymap_cmd {
            return Effect::Run(cmd);
        }

        // First `g` of `gg` and unrecognised keys both land here. The
        // background is always "visible" so the router treats this as
        // a no-op (no AppCommand, no auto-release).
        Effect::Pass
    }

    /// The background is the one surface that is *always* available —
    /// it is the resting state of the focus manager and is never
    /// released. Overrides the trait default (`false`) which is the
    /// safe assumption for everything else.
    fn is_visible(&self) -> bool {
        true
    }

    /// Footer hints shown when the background owns focus (i.e. no
    /// modal is up). The cycle hint (`Tab/S-Tab`) is *not* included
    /// here — it depends on whether any plugin is currently visible,
    /// which the background can't see. The UI layer adds it when
    /// applicable.
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
    use crate::geo::LonLat;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: SurfaceCtx = SurfaceCtx {
        center: LonLat { lon: 0.0, lat: 0.0 },
        theme_id: crate::color_palette::ThemeId::Dark,
    };

    fn map(action: Action) -> AppCommand {
        AppCommand::Map(action)
    }

    fn bg() -> BackgroundResponder {
        BackgroundResponder::new(KeyMap::default(), Vec::new())
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut bg = bg();
        // 1st g: nothing fires, pending_g latched.
        assert_eq!(bg.handle_key(KeyCode::Char('g'), NONE, CTX), Effect::Pass);
        // 2nd g: ZoomToWorld.
        assert_eq!(
            bg.handle_key(KeyCode::Char('g'), NONE, CTX),
            Effect::Run(map(Action::ZoomToWorld))
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut bg = bg();
        bg.handle_key(KeyCode::Char('g'), NONE, CTX);
        bg.handle_key(KeyCode::Char('h'), NONE, CTX); // breaks
        // Now pending_g was reset; this g latches afresh, doesn't fire.
        assert_eq!(bg.handle_key(KeyCode::Char('g'), NONE, CTX), Effect::Pass);
    }
}
