//! Keyboard input handler. Translates raw key events into `Action`s
//! (handling the `gg` sequence + non-remappable mode transitions),
//! then orchestrates dispatch to widgets and core.
//!
//! Key and mouse paths stay intentionally separate — they have
//! different semantics (keys are modal/captured, mouse is observer +
//! target), matching the pattern used by helix and other Rust TUI
//! apps (gitui documented a regret for unifying them).

use crossterm::event::{KeyCode, KeyModifiers};
use log::info;

use crate::app::InputEffect;
use crate::core::keymap::KeyMap;
use crate::core::{Action, Core};
use crate::ui::UiState;
use crate::ui::widget::WidgetAction;

pub struct KeyboardHandler {
    pending_g: bool,
    keymap: KeyMap,
}

impl KeyboardHandler {
    pub fn new(keymap: KeyMap) -> Self {
        Self {
            pending_g: false,
            keymap,
        }
    }

    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    pub fn handle(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        core: &mut Core,
        ui: &mut UiState,
    ) -> InputEffect {
        let center = core.center();

        // Raw-key pass: let active widgets consume or Jump.
        for widget in ui.widgets_mut() {
            match widget.handle_key(code, modifiers, center) {
                WidgetAction::Pass => continue,
                WidgetAction::Consumed => return InputEffect::Widget,
                WidgetAction::Jump(location) => {
                    info!("widget: jumping to ({}, {})", location.lat, location.lon);
                    core.jump_to(location);
                    return InputEffect::Map;
                }
            }
        }

        // Translate to a global Action, then let widgets claim it
        // (SearchOpen, HelpToggle, WikiToggle), then fall through to core.
        let action = self.resolve_action(code, modifiers);
        for widget in ui.widgets_mut() {
            if widget.handle_action(&action, center) {
                return InputEffect::Widget;
            }
        }
        if core.process_action(&action) {
            InputEffect::Map
        } else {
            InputEffect::None
        }
    }

    /// Raw key → `Action` translation. Handles the `gg` two-char
    /// sequence, non-remappable mode transitions (`/`, `?`, `i`), and
    /// finally falls back to the configured keymap.
    fn resolve_action(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Action {
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

        if code == KeyCode::Char('/') && modifiers == KeyModifiers::NONE {
            return Action::SearchOpen;
        }
        if code == KeyCode::Char('?') && modifiers == KeyModifiers::NONE {
            return Action::HelpToggle;
        }
        if code == KeyCode::Char('i') && modifiers == KeyModifiers::NONE {
            return Action::WikiToggle;
        }

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

    fn handler() -> KeyboardHandler {
        KeyboardHandler::new(KeyMap::default())
    }

    #[test]
    fn test_basic_movement() {
        let mut h = handler();
        assert_eq!(h.resolve_action(KeyCode::Char('h'), NONE), Action::PanLeft);
        assert_eq!(h.resolve_action(KeyCode::Char('j'), NONE), Action::PanDown);
        assert_eq!(h.resolve_action(KeyCode::Char('k'), NONE), Action::PanUp);
        assert_eq!(h.resolve_action(KeyCode::Char('l'), NONE), Action::PanRight);
    }

    #[test]
    fn test_gg_zoom_to_world() {
        let mut h = handler();
        h.resolve_action(KeyCode::Char('g'), NONE);
        assert_eq!(
            h.resolve_action(KeyCode::Char('g'), NONE),
            Action::ZoomToWorld
        );
    }

    #[test]
    fn test_zoom() {
        let mut h = handler();
        assert_eq!(h.resolve_action(KeyCode::Char('a'), NONE), Action::ZoomIn);
        assert_eq!(h.resolve_action(KeyCode::Char('z'), NONE), Action::ZoomOut);
    }

    #[test]
    fn test_quit() {
        let mut h = handler();
        assert_eq!(h.resolve_action(KeyCode::Char('q'), NONE), Action::Quit);
    }

    #[test]
    fn test_big_pan() {
        let mut h = handler();
        assert_eq!(
            h.resolve_action(KeyCode::Char('w'), NONE),
            Action::PanRightFast
        );
        assert_eq!(
            h.resolve_action(KeyCode::Char('b'), NONE),
            Action::PanLeftFast
        );
        assert_eq!(
            h.resolve_action(KeyCode::Char('d'), KeyModifiers::CONTROL),
            Action::PanDownHalf
        );
        assert_eq!(
            h.resolve_action(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Action::PanUpHalf
        );
    }

    #[test]
    fn test_reset_position() {
        let mut h = handler();
        assert_eq!(
            h.resolve_action(KeyCode::Char('0'), NONE),
            Action::ResetPosition
        );
    }
}
