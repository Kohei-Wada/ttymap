//! Key binding types, default bindings, and key notation parser.

use crossterm::event::{KeyCode, KeyModifiers};

use super::input::Action;

/// A key binding: a key code + optional modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

pub struct KeyMap {
    pub bindings: Vec<(KeyBinding, Action)>,
}

impl KeyMap {
    /// Look up the action for a key event. Returns None if no binding matches.
    pub fn lookup(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&Action> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.bindings
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, a)| a)
    }

    /// Replace every existing binding for `action` with the supplied
    /// list of key strings (e.g. `["h", "Left"]`). Invalid key strings
    /// are logged and skipped. Used by the app layer when applying
    /// `[keymap]` overrides from config.
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
        }
    }
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
}
