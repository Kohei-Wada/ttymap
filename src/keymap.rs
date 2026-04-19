//! Key binding table — the `key → Action` data used by the keyboard
//! handler, plus the TOML-deserialisable `KeybindingOverrides` shape
//! used by config to customise it.

use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

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
    pub bindings: Vec<(KeyBinding, Action)>,
    /// First-`g`-of-`gg` flag. Held on the map, not the keyboard
    /// handler, so all key→`Action` translation stays in one place.
    pending_g: bool,
}

impl KeyMap {
    /// Look up the action for a key event. Returns None if no binding
    /// matches. Stateless — ignores sequence state.
    pub fn lookup(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&Action> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.bindings
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, a)| a)
    }

    /// Resolve a key event to an `Action`. Handles the `gg` sequence
    /// ahead of user-configurable bindings. Returns `Action::None`
    /// while mid-sequence or when the key has no binding. Plugin
    /// activation (e.g. `/` opens search) is **not** handled here —
    /// widgets own their activation keys and the keyboard handler
    /// checks them before falling through to this resolver.
    pub fn resolve(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Action {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Action::ZoomToWorld;
            }
            self.pending_g = true;
            return Action::None;
        }
        self.pending_g = false;

        self.lookup(code, modifiers)
            .cloned()
            .unwrap_or(Action::None)
    }

    /// Every key string currently bound to `action`, in registration
    /// order. Used by the command palette and help overlay to show
    /// "this command is invocable via these keys" hints.
    pub fn keys_for(&self, action: &Action) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, a)| a == action)
            .map(|(b, _)| b.display())
            .collect()
    }

    /// Replace every existing binding for `action` with the supplied
    /// list of key strings (e.g. `["h", "Left"]`). Invalid key strings
    /// are logged and skipped.
    pub fn set_bindings(&mut self, action: Action, keys: &[String]) {
        self.bindings.retain(|(_, a)| a != &action);
        for key_str in keys {
            if let Some(binding) = parse_key_binding(key_str) {
                self.bindings.push((binding, action.clone()));
            } else {
                log::warn!("invalid key binding: {:?}", key_str);
            }
        }
    }

    /// Default bindings with user `[keymap]` overrides applied on top.
    pub fn with_overrides(overrides: &KeybindingOverrides) -> Self {
        let mut km = Self::default();

        macro_rules! rebind {
            ($field:ident, $action:expr) => {
                if let Some(keys) = &overrides.$field {
                    km.set_bindings($action, keys);
                }
            };
        }

        rebind!(pan_left, Action::PanLeft);
        rebind!(pan_right, Action::PanRight);
        rebind!(pan_up, Action::PanUp);
        rebind!(pan_down, Action::PanDown);
        rebind!(pan_left_fast, Action::PanLeftFast);
        rebind!(pan_right_fast, Action::PanRightFast);
        rebind!(pan_up_half, Action::PanUpHalf);
        rebind!(pan_down_half, Action::PanDownHalf);
        rebind!(zoom_in, Action::ZoomIn);
        rebind!(zoom_out, Action::ZoomOut);
        rebind!(zoom_to_world, Action::ZoomToWorld);
        rebind!(reset_position, Action::ResetPosition);
        rebind!(quit, Action::Quit);

        km
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        use Action::*;
        let b = |key: &str, action: Action| -> (KeyBinding, Action) {
            (parse_key_binding(key).unwrap(), action)
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
            pending_g: false,
        }
    }
}

/// Raw keybinding overrides from the `[keymap]` section of
/// `config.toml`. Each field names an `Action`; the listed key strings
/// replace the default bindings for that action. Applied via
/// `KeyMap::with_overrides`.
#[derive(Deserialize, Default, Clone)]
pub struct KeybindingOverrides {
    pub pan_left: Option<Vec<String>>,
    pub pan_right: Option<Vec<String>>,
    pub pan_up: Option<Vec<String>>,
    pub pan_down: Option<Vec<String>>,
    pub pan_left_fast: Option<Vec<String>>,
    pub pan_right_fast: Option<Vec<String>>,
    pub pan_up_half: Option<Vec<String>>,
    pub pan_down_half: Option<Vec<String>>,
    pub zoom_in: Option<Vec<String>>,
    pub zoom_out: Option<Vec<String>>,
    pub zoom_to_world: Option<Vec<String>>,
    pub reset_position: Option<Vec<String>>,
    pub quit: Option<Vec<String>>,
}

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
            Some(&Action::PanLeft)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(&Action::PanDownHalf)
        );
        assert_eq!(km.lookup(KeyCode::Char('x'), KeyModifiers::NONE), None);
    }

    #[test]
    fn with_overrides_applies_rebinds() {
        let mut overrides = KeybindingOverrides::default();
        overrides.zoom_in = Some(vec!["i".to_string()]);
        overrides.quit = Some(vec!["Q".to_string(), "C-q".to_string()]);

        let km = KeyMap::with_overrides(&overrides);

        assert_eq!(
            km.lookup(KeyCode::Char('i'), KeyModifiers::NONE),
            Some(&Action::ZoomIn)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('Q'), KeyModifiers::NONE),
            Some(&Action::Quit)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(&Action::Quit)
        );
    }

    #[test]
    fn with_overrides_keeps_unoverridden_defaults() {
        let km = KeyMap::with_overrides(&KeybindingOverrides::default());
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&Action::PanLeft)
        );
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;

    #[test]
    fn resolve_basic_movement() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('h'), NONE), Action::PanLeft);
        assert_eq!(km.resolve(KeyCode::Char('j'), NONE), Action::PanDown);
        assert_eq!(km.resolve(KeyCode::Char('k'), NONE), Action::PanUp);
        assert_eq!(km.resolve(KeyCode::Char('l'), NONE), Action::PanRight);
    }

    #[test]
    fn resolve_gg_zoom_to_world() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('g'), NONE), Action::None);
        assert_eq!(km.resolve(KeyCode::Char('g'), NONE), Action::ZoomToWorld);
    }

    #[test]
    fn resolve_gg_sequence_broken_by_other_key() {
        let mut km = KeyMap::default();
        km.resolve(KeyCode::Char('g'), NONE);
        km.resolve(KeyCode::Char('h'), NONE); // breaks sequence
        assert_eq!(km.resolve(KeyCode::Char('g'), NONE), Action::None);
    }

    #[test]
    fn resolve_zoom() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('a'), NONE), Action::ZoomIn);
        assert_eq!(km.resolve(KeyCode::Char('z'), NONE), Action::ZoomOut);
    }

    #[test]
    fn resolve_quit() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('q'), NONE), Action::Quit);
    }

    #[test]
    fn resolve_big_pan() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('w'), NONE), Action::PanRightFast);
        assert_eq!(km.resolve(KeyCode::Char('b'), NONE), Action::PanLeftFast);
        assert_eq!(
            km.resolve(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::PanDownHalf
        );
        assert_eq!(
            km.resolve(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Action::PanUpHalf
        );
    }

    #[test]
    fn resolve_reset_position() {
        let mut km = KeyMap::default();
        assert_eq!(km.resolve(KeyCode::Char('0'), NONE), Action::ResetPosition);
    }

    #[test]
    fn resolve_unknown_key_is_none() {
        let mut km = KeyMap::default();
        // `/`, `?`, `i` are widget activation triggers, not keymap
        // entries — they fall through to `Action::None`.
        assert_eq!(km.resolve(KeyCode::Char('/'), NONE), Action::None);
        assert_eq!(km.resolve(KeyCode::Char('?'), NONE), Action::None);
        assert_eq!(km.resolve(KeyCode::Char('i'), NONE), Action::None);
    }
}
