//! Key binding table — the `key → UserCommand` data used by the
//! keyboard handler, plus `KeyMap::with_overrides` which folds in
//! the [`KeybindingOverrides`] settings map that `ttymap.keymap.set/del`
//! populates from `init.lua`.
//!
//! The keymap speaks the same [`UserCommand`] vocabulary as the
//! palette and plugins — every key binding resolves to a
//! `UserCommand` that the binary's event loop dispatches. Today all
//! defaults are `UserCommand::Map` wrappers, but nothing prevents
//! binding a key to `UserCommand::SetTheme(...)` or
//! `UserCommand::CycleFocus(...)` in the future. (Surface
//! activations like opening the palette or a plugin are *not*
//! `UserCommand`s — those go through the compositor's
//! [`crate::compositor::window::Window`] queue, applied atomically
//! after the [`crate::compositor::BaseLayer`] hook returns.)
//!
//! Lives in `ttymap-tui` (not `ttymap-shared`) so the crossterm
//! dependency that backs [`KeyCode`] / [`KeyModifiers`] stays out
//! of the cross-cutting vocabulary crate. The user-facing
//! [`KeybindingOverrides`] settings type lives in `ttymap-config`
//! alongside the rest of the runtime config shape; this module
//! imports it to fold overrides into a live table.

use crossterm::event::{KeyCode, KeyModifiers};

use ttymap_config::KeybindingOverrides;
use ttymap_engine::map::MapAction;
use ttymap_shared::UserCommand;

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
    pub bindings: Vec<(KeyBinding, UserCommand)>,
}

impl KeyMap {
    /// Look up the command for a key event. Returns `None` if no
    /// binding matches. Stateless — multi-key sequences (e.g. `gg`)
    /// are owned by the keyboard handler, not the keymap.
    pub fn lookup(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&UserCommand> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.bindings
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, c)| c)
    }

    /// Resolve a key event to a `UserCommand`. Stateless wrapper around
    /// [`lookup`](Self::lookup) that clones for ownership. Plugin
    /// activation (e.g. `/` opens search) is **not** handled here —
    /// widgets own their activation keys and the keyboard handler
    /// checks them before falling through to this resolver.
    pub fn resolve(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<UserCommand> {
        self.lookup(code, modifiers).cloned()
    }

    /// Every key string currently bound to `cmd`, in registration
    /// order. Used by the command palette and help overlay to show
    /// "this command is invocable via these keys" hints.
    pub fn keys_for(&self, cmd: &UserCommand) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, c)| c == cmd)
            .map(|(b, _)| b.display())
            .collect()
    }

    /// Replace every existing binding for `cmd` with the supplied list
    /// of key strings (e.g. `["h", "Left"]`). Invalid key strings are
    /// logged and skipped.
    pub fn set_bindings(&mut self, cmd: UserCommand, keys: &[String]) {
        self.bindings.retain(|(_, c)| c != &cmd);
        for key_str in keys {
            if let Some(binding) = parse_key_binding(key_str) {
                self.bindings.push((binding, cmd.clone()));
            } else {
                log::warn!("invalid key binding: {:?}", key_str);
            }
        }
    }

    /// Default bindings with user `[keymap]` overrides applied on
    /// top. Each entry's key is a [`UserCommand`] config name (e.g.
    /// `"pan_left"`, `"quit"`); unknown names are logged and skipped
    /// so a stale config can't crash startup.
    pub fn with_overrides(overrides: &KeybindingOverrides) -> Self {
        let mut km = Self::default();
        for (name, keys) in overrides {
            match UserCommand::from_config_name(name) {
                Some(intent) => km.set_bindings(intent, keys),
                None => log::warn!("unknown [keymap] entry: {:?}", name),
            }
        }
        km
    }

    /// Help-table entries: `(keys, label)` strings for every listed
    /// command that has at least one key bound. Drives the
    /// `ttymap.help:keymap_entries()` Lua surface.
    pub fn help_entries(&self) -> Vec<(String, String)> {
        UserCommand::listed_with_labels()
            .into_iter()
            .filter_map(|(cmd, label)| {
                let keys = self.keys_for(&cmd);
                if keys.is_empty() {
                    None
                } else {
                    Some((keys.join(", "), label.to_string()))
                }
            })
            .collect()
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        use MapAction::*;
        let map = |key: &str, action: MapAction| -> (KeyBinding, UserCommand) {
            (parse_key_binding(key).unwrap(), UserCommand::Map(action))
        };
        let intent = |key: &str, intent: UserCommand| -> (KeyBinding, UserCommand) {
            (parse_key_binding(key).unwrap(), intent)
        };
        Self {
            bindings: vec![
                map("h", PanLeft),
                map("Left", PanLeft),
                map("l", PanRight),
                map("Right", PanRight),
                map("k", PanUp),
                map("Up", PanUp),
                map("j", PanDown),
                map("Down", PanDown),
                map("b", PanLeftFast),
                map("w", PanRightFast),
                map("C-u", PanUpHalf),
                map("C-d", PanDownHalf),
                map("a", ZoomIn),
                map("+", ZoomIn),
                map("z", ZoomOut),
                map("-", ZoomOut),
                map("0", ResetPosition),
                intent("q", UserCommand::Quit),
                intent("\\", UserCommand::ToggleSidebar),
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

    fn map(action: MapAction) -> UserCommand {
        UserCommand::Map(action)
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
            Some(&map(MapAction::PanLeft))
        );
        assert_eq!(
            km.lookup(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(&map(MapAction::PanDownHalf))
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
            Some(&map(MapAction::ZoomIn))
        );
        assert_eq!(
            km.lookup(KeyCode::Char('Q'), KeyModifiers::NONE),
            Some(&UserCommand::Quit)
        );
        assert_eq!(
            km.lookup(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(&UserCommand::Quit)
        );
    }

    #[test]
    fn with_overrides_keeps_unoverridden_defaults() {
        let km = KeyMap::with_overrides(&KeybindingOverrides::new());
        assert_eq!(
            km.lookup(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(&map(MapAction::PanLeft))
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
            Some(&map(MapAction::PanLeft))
        );
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;

    #[test]
    fn resolve_basic_movement() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('h'), NONE),
            Some(map(MapAction::PanLeft))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('j'), NONE),
            Some(map(MapAction::PanDown))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('k'), NONE),
            Some(map(MapAction::PanUp))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('l'), NONE),
            Some(map(MapAction::PanRight))
        );
    }

    #[test]
    fn resolve_zoom() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('a'), NONE),
            Some(map(MapAction::ZoomIn))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('z'), NONE),
            Some(map(MapAction::ZoomOut))
        );
    }

    #[test]
    fn resolve_quit() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('q'), NONE),
            Some(UserCommand::Quit)
        );
    }

    #[test]
    fn resolve_big_pan() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('w'), NONE),
            Some(map(MapAction::PanRightFast))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('b'), NONE),
            Some(map(MapAction::PanLeftFast))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Some(map(MapAction::PanDownHalf))
        );
        assert_eq!(
            km.resolve(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Some(map(MapAction::PanUpHalf))
        );
    }

    #[test]
    fn resolve_reset_position() {
        let km = KeyMap::default();
        assert_eq!(
            km.resolve(KeyCode::Char('0'), NONE),
            Some(map(MapAction::ResetPosition))
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
