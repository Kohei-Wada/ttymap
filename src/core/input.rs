use crossterm::event::{KeyCode, KeyModifiers};

use super::keymap::KeyMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    PanUp,
    PanDown,
    PanLeft,
    PanRight,
    PanLeftFast,
    PanRightFast,
    PanUpHalf,
    PanDownHalf,
    ZoomIn,
    ZoomOut,
    ZoomToWorld,
    ResetPosition,
    Redraw,
    SearchOpen,
    HelpToggle,
    WikiToggle,
}

pub struct InputHandler {
    pending_g: bool,
    keymap: KeyMap,
}

impl InputHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            pending_g: false,
            keymap,
        }
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Action {
        // 'gg' sequence for ZoomToWorld
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Action::ZoomToWorld;
            } else {
                self.pending_g = true;
                return Action::None;
            }
        }
        self.pending_g = false;

        // Mode transitions (not remappable)
        if code == KeyCode::Char('/') && modifiers == KeyModifiers::NONE {
            return Action::SearchOpen;
        }
        if code == KeyCode::Char('?') && modifiers == KeyModifiers::NONE {
            return Action::HelpToggle;
        }
        if code == KeyCode::Char('i') && modifiers == KeyModifiers::NONE {
            return Action::WikiToggle;
        }

        // Keymap lookup
        if let Some(action) = self.keymap.lookup(code, modifiers) {
            return action.clone();
        }

        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn handler() -> InputHandler {
        InputHandler::new(KeyMap::default())
    }

    #[test]
    fn test_basic_movement() {
        let mut h = handler();
        assert_eq!(h.handle_key(KeyCode::Char('h'), NONE), Action::PanLeft);
        assert_eq!(h.handle_key(KeyCode::Char('j'), NONE), Action::PanDown);
        assert_eq!(h.handle_key(KeyCode::Char('k'), NONE), Action::PanUp);
        assert_eq!(h.handle_key(KeyCode::Char('l'), NONE), Action::PanRight);
    }

    #[test]
    fn test_gg_zoom_to_world() {
        let mut h = handler();
        h.handle_key(KeyCode::Char('g'), NONE);
        assert_eq!(h.handle_key(KeyCode::Char('g'), NONE), Action::ZoomToWorld);
    }

    #[test]
    fn test_zoom() {
        let mut h = handler();
        assert_eq!(h.handle_key(KeyCode::Char('a'), NONE), Action::ZoomIn);
        assert_eq!(h.handle_key(KeyCode::Char('z'), NONE), Action::ZoomOut);
    }

    #[test]
    fn test_quit() {
        let mut h = handler();
        assert_eq!(h.handle_key(KeyCode::Char('q'), NONE), Action::Quit);
    }

    #[test]
    fn test_big_pan() {
        let mut h = handler();
        assert_eq!(h.handle_key(KeyCode::Char('w'), NONE), Action::PanRightFast);
        assert_eq!(h.handle_key(KeyCode::Char('b'), NONE), Action::PanLeftFast);
        assert_eq!(
            h.handle_key(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::PanDownHalf
        );
        assert_eq!(
            h.handle_key(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Action::PanUpHalf
        );
    }

    #[test]
    fn test_reset_position() {
        let mut h = handler();
        assert_eq!(
            h.handle_key(KeyCode::Char('0'), NONE),
            Action::ResetPosition
        );
    }
}
