//! Key binding table — the `key → UserIntent` data used by the keyboard
//! handler, plus the TOML-deserialisable `KeybindingOverrides` shape
//! used by config to customise it.
//!
//! The keymap speaks the same `UserIntent` vocabulary as the palette
//! and plugins — every key binding resolves to a `UserIntent` that
//! rides through [`Frontend::dispatch`](crate::frontend::Frontend). Today all
//! defaults are `UserIntent::Map` wrappers, but nothing prevents binding
//! a key to `UserIntent::SetTheme(...)` or `UserIntent::CycleFocus(...)` in
//! the future. (Surface activations like opening the palette or a plugin
//! are *not* `UserIntent`s — those go through the compositor's
//! [`Window::open`](crate::frontend::compositor::window::Window) / `toggle`
//! queue, applied atomically after the `BaseLayer` hook returns.)

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::frontend::UserIntent;
use crate::map::Action;

/// A key binding: a key code + optional modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    /// Human-readable form for UI display (help overlay, command
    /// palette). Matches the input notation accepted by
    /// [`parse_key_binding`] for plain keys (`"h"`, `"Left"`, `"C-d"`).
    pub fn display(&self) -> String {
        let key = match self.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Backspace => "BS".to_string(),
            _ => "?".to_string(),
        };

        if self.modifiers.contains(KeyModifiers::CONTROL) {
            format!("C-{}", key)
        } else if self.modifiers.contains(KeyModifiers::SHIFT) {
            format!("S-{}", key)
        } else {
            key
        }
    }
}

pub struct KeyMap {
    pub bindings: Vec<(KeyBinding, UserIntent)>,
}

impl KeyMap {
    /// Look up the command for a key event. Returns `None` if no
    /// binding matches. Stateless — multi-key sequences (e.g. `gg`)
    /// are owned by the keyboard handler, not the keymap.
    pub fn lookup(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&UserIntent> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.bindings
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, c)| c)
    }

    /// Resolve a key event to a `UserIntent`. Stateless wrapper around
    /// [`lookup`] that clones for ownership. Plugin activation (e.g.
    /// `/` opens search) is **not** handled here — widgets own their
    /// activation keys and the keyboard handler checks them before
    /// falling through to this resolver.
    pub fn resolve(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<UserIntent> {
        self.lookup(code, modifiers).cloned()
    }

    /// Every key string currently bound to `cmd`, in registration
    /// order. Used by the command palette and help overlay to show
    /// "this command is invocable via these keys" hints.
    pub fn keys_for(&self, cmd: &UserIntent) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, c)| c == cmd)
            .map(|(b, _)| b.display())
            .collect()
    }

    /// Replace every existing binding for `cmd` with the supplied list
    /// of key strings (e.g. `["h", "Left"]`). Invalid key strings are
    /// logged and skipped.
    pub fn set_bindings(&mut self, cmd: UserIntent, keys: &[String]) {
        self.bindings.retain(|(_, c)| c != &cmd);
        for key_str in keys {
            if let Some(binding) = parse_key_binding(key_str) {
                self.bindings.push((binding, cmd.clone()));
            } else {
                log::warn!("invalid key binding: {:?}", key_str);
            }
        }
    }

    /// Default bindings with user `[keymap]` overrides applied on top.
    /// Each entry's key is the `Action::config_name` (e.g. `"pan_left"`);
    /// unknown names are logged and skipped so a stale config can't
    /// crash startup.
    pub fn with_overrides(overrides: &KeybindingOverrides) -> Self {
        let mut km = Self::default();
        for (name, keys) in overrides {
            match Action::from_config_name(name) {
                Some(action) => km.set_bindings(UserIntent::Map(action), keys),
                None => log::warn!("unknown [keymap] entry: {:?}", name),
            }
        }
        km
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        use Action::*;
        let b = |key: &str, action: Action| -> (KeyBinding, UserIntent) {
            (parse_key_binding(key).unwrap(), UserIntent::Map(action))
        };
        Self {
            bindings: vec![
                b("h", PanLeft),
                b("Left", PanLeft),
                b("l", PanRight),
                b("Right", PanRight),
                b("k", PanUp),
                b("Up", PanUp),
                b("j", PanDown),
                b("Down", PanDown),
                b("b", PanLeftFast),
                b("w", PanRightFast),
                b("C-u", PanUpHalf),
                b("C-d", PanDownHalf),
                b("a", ZoomIn),
                b("+", ZoomIn),
                b("z", ZoomOut),
                b("-", ZoomOut),
                b("0", ResetPosition),
                b("q", Quit),
            ],
        }
    }
}

/// Raw keybinding overrides from the `[keymap]` section of
/// `config.toml`. Keys are `Action::config_name` strings (e.g.
/// `"pan_left"`); values replace the default bindings for that
/// action (wrapped as `UserIntent::Map` internally). Applied via
/// `KeyMap::with_overrides`. Adding a new bindable `Action` only
/// requires extending `Action::all_listed` + `Action::config_name`
/// — the data shape here is unchanged.
pub type KeybindingOverrides = HashMap<String, Vec<String>>;

// ── Key binding parser ────────────────────────────────────────────────────────

/// Parse a key binding string like "h", "Left", "C-d".
/// Notation: "C-x" for Ctrl+x, "S-x" for Shift+x, "Left"/"Enter"/etc for special keys.
pub fn parse_key_binding(s: &str) -> Option<KeyBinding> {
    if let Some(ch) = s.strip_prefix("C-") {
        let code = parse_key_code(ch)?;
        Some(KeyBinding {
            code,
            modifiers: KeyModifiers::CONTROL,
        })
    } else if let Some(ch) = s.strip_prefix("S-") {
        let code = parse_key_code(ch)?;
        Some(KeyBinding {
            code,
            modifiers: KeyModifiers::SHIFT,
        })
    } else {
        let code = parse_key_code(s)?;
        Some(KeyBinding {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }
}

fn parse_key_code(s: &str) -> Option<KeyCode> {
    match s {
        "Left" => Some(KeyCode::Left),
        "Right" => Some(KeyCode::Right),
        "Up" => Some(KeyCode::Up),
        "Down" => Some(KeyCode::Down),
        "Enter" => Some(KeyCode::Enter),
        "Esc" => Some(KeyCode::Esc),
        "Tab" => Some(KeyCode::Tab),
        "Backspace" => Some(KeyCode::Backspace),
        s if s.len() == 1 => Some(KeyCode::Char(s.chars().next().unwrap())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(action: Action) -> UserIntent {
        UserIntent::Map(action)
    }

    #[test]
    fn test_parse_char() {
        let b = parse_key_binding("h").unwrap();
        assert_eq!(b.code, KeyCode::Char('h'));
        assert_eq!(b.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_ctrl() {
        let b = parse_key_binding("C-d").unwrap();
        assert_eq!(b.code, KeyCode::Char('d'));
        assert_eq!(b.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_parse_special() {
        assert_eq!(parse_key_binding("Left").unwrap().code, KeyCode::Left);
        assert_eq!(
            parse_key_binding("Backspace").unwrap().code,
            KeyCode::Backspace
        );
        assert_eq!(parse_key_binding("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key_binding("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key_binding("Tab").unwrap().code, KeyCode::Tab);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_key_binding("InvalidKey").is_none());
        assert!(parse_key_binding("").is_none());
    }

    #[test]
    fn test_lookup() {
        let km = KeyMap::default();
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&map(Action::PanLeft))
        );
        assert_eq!(
            km.lookup(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(&map(Action::PanDownHalf))
        );
        assert_eq!(km.lookup(KeyCode::Char('x'), KeyModifiers::NONE), None);
    }

    #[test]
    fn with_overrides_applies_rebinds() {
        let mut overrides = KeybindingOverrides::new();
        overrides.insert("zoom_in".to_string(), vec!["i".to_string()]);
        overrides.insert("quit".to_string(), vec!["Q".to_string(), "C-q".to_string()]);

        let km = KeyMap::with_overrides(&overrides);

        assert_eq!(
            km.lookup(KeyCode::Char('i'), KeyModifiers::NONE),
            Some(&map(Action::ZoomIn))
        );
        assert_eq!(
            km.lookup(KeyCode::Char('Q'), KeyModifiers::NONE),
            Some(&map(Action::Quit))
        );
        assert_eq!(
            km.lookup(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(&map(Action::Quit))
        );
    }

    #[test]
    fn with_overrides_keeps_unoverridden_defaults() {
        let km = KeyMap::with_overrides(&KeybindingOverrides::new());
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&map(Action::PanLeft))
        );
    }

    #[test]
    fn with_overrides_skips_unknown_entries() {
        let mut overrides = KeybindingOverrides::new();
        overrides.insert("not_an_action".to_string(), vec!["x".to_string()]);
        // Defaults survive an unknown entry — it must not poison the
        // rest of the table.
        let km = KeyMap::with_overrides(&overrides);
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&map(Action::PanLeft))
        );
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;

    #[test]
    fn resolve_basic_movement() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('h'), NONE),
            Some(map(Action::PanLeft))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('j'), NONE),
            Some(map(Action::PanDown))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('k'), NONE),
            Some(map(Action::PanUp))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('l'), NONE),
            Some(map(Action::PanRight))
        );
    }

    #[test]
    fn resolve_zoom() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('a'), NONE),
            Some(map(Action::ZoomIn))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('z'), NONE),
            Some(map(Action::ZoomOut))
        );
    }

    #[test]
    fn resolve_quit() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('q'), NONE),
            Some(map(Action::Quit))
        );
    }

    #[test]
    fn resolve_big_pan() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('w'), NONE),
            Some(map(Action::PanRightFast))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('b'), NONE),
            Some(map(Action::PanLeftFast))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(map(Action::PanDownHalf))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Some(map(Action::PanUpHalf))
        );
    }

    #[test]
    fn resolve_reset_position() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('0'), NONE),
            Some(map(Action::ResetPosition))
        );
    }

    #[test]
    fn resolve_unknown_key_is_none() {
        let km = KeyMap::default();
        // `/`, `?`, `i` are widget activation triggers, not keymap
        // entries — they fall through to `None`.
        assert_eq!(km.resolve(KeyCode::Char('/'), NONE), None);
        assert_eq!(km.resolve(KeyCode::Char('?'), NONE), None);
        assert_eq!(km.resolve(KeyCode::Char('i'), NONE), None);
    }
}
