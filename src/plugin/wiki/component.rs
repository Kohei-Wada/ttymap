//! Wiki component — side panel + map markers, pushed onto the
//! compositor stack while the plugin is toggled on.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::plugin_api::prelude::*;

use super::panel;
use super::state::WikiHandle;

/// Wiki component — owns no state of its own. All state (articles,
/// selection, detail view) lives in the shared [`WikiHandle`], so
/// push/pop is cheap and the next open inherits the prior list.
pub struct WikiComponent {
    state: WikiHandle,
}

impl WikiComponent {
    pub fn new(state: WikiHandle, center: LonLat) -> Self {
        // Trigger a refresh on (re)open so the list reflects the
        // user's current position. Replaces the old `activate` hook.
        state.borrow_mut().refresh(center);
        Self { state }
    }
}

impl Component for WikiComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let mut state = self.state.borrow_mut();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        // Self-toggle on the activation key.
        if event.code == KeyCode::Char('i') && event.modifiers == KeyModifiers::NONE {
            win.close();
            return;
        }

        // Refresh is available even when the list is empty.
        if event.code == KeyCode::Char('r') {
            state.refresh(win.ctx().center);
            return;
        }

        let up = (ctrl && event.code == KeyCode::Char('p')) || event.code == KeyCode::Up;
        let down = (ctrl && event.code == KeyCode::Char('n')) || event.code == KeyCode::Down;
        let exit_detail = matches!(
            event.code,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter
        );

        if state.articles.is_empty() {
            // Panel is open but has nothing yet — still swallow
            // widget-control keys so they don't fall through to the
            // keymap, but let everything else pass through (non-
            // modal behaviour).
            if !(up || down || exit_detail) {
                win.ignore();
            }
            return;
        }

        // ── Detail mode ─────────────────────────────────────────────
        if state.is_detail_open() {
            if exit_detail {
                state.detail = None;
                return;
            }
            if up || down {
                let n = state.articles.len();
                state.selected = if up {
                    if state.selected == 0 {
                        n - 1
                    } else {
                        state.selected - 1
                    }
                } else {
                    (state.selected + 1) % n
                };
                let article = state.articles[state.selected].clone();
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                state.detail = Some(article);
                win.emit(AppMsg::Jump(loc));
            }
            return;
        }

        // ── List mode ───────────────────────────────────────────────
        if event.code == KeyCode::Enter {
            if let Some(article) = state.articles.get(state.selected) {
                let loc = LonLat {
                    lat: article.lat,
                    lon: article.lon,
                };
                state.detail = Some(article.clone());
                win.emit(AppMsg::Jump(loc));
            }
            return;
        }
        if up || down {
            let n = state.articles.len();
            state.selected = if up {
                if state.selected == 0 {
                    n - 1
                } else {
                    state.selected - 1
                }
            } else {
                (state.selected + 1) % n
            };
            let article = &state.articles[state.selected];
            win.emit(AppMsg::Jump(LonLat {
                lat: article.lat,
                lon: article.lon,
            }));
            return;
        }
        if matches!(event.code, KeyCode::Esc | KeyCode::Backspace) {
            return;
        }

        // Non-modal: let lower layers handle unknown keys.
        win.ignore();
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(&self.state.borrow(), win);
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        let state = self.state.borrow();
        let primary = p.accent_color();
        let highlight = p.accent_alt_color();
        for (i, a) in state.articles.iter().enumerate() {
            let fg = if i == state.selected {
                highlight
            } else {
                primary
            };
            p.point(
                LonLat {
                    lon: a.lon,
                    lat: a.lat,
                },
                '●',
                fg,
            );
        }
    }

    fn poll(&mut self, _win: &mut Window) {
        self.state.borrow_mut().poll();
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.borrow().is_detail_open() {
            vec![
                ("C-n/C-p", "prev/next"),
                ("Enter/Esc", "back"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("?", "help"),
            ]
        } else {
            vec![
                ("C-n/C-p", "select"),
                ("Enter", "open"),
                ("r", "refresh"),
                ("i", "close wiki"),
                ("/", "search"),
                ("?", "help"),
            ]
        }
    }

    fn name(&self) -> &'static str {
        "wiki"
    }
}
