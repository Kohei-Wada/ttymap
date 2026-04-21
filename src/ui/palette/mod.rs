//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! # Deliberately not a `Plugin` — do not "unify"
//!
//! Plugins and the palette look similar at a distance (both react to
//! key events, both draw popups), but they're different categories:
//!
//! - **Plugin** = a component that *contributes* functionality (one
//!   feature, self-contained state, small activation key surface).
//! - **Palette** = a *coordinator* that aggregates over the plugin
//!   registry + keymap + theme state to present a unified picker.
//!   Its whole job is to read other subsystems' state.
//!
//! Folding palette into the `Plugin` trait would work mechanically —
//! with a wider `PluginCtx<'a>` carrying `&PluginRegistry`, `&KeyMap`,
//! `ThemeId` — but it would erase that semantic distinction: every
//! plugin would gain permission to enumerate the registry (the "plugins
//! don't see each other" invariant weakens from `pub(crate)` structure
//! to a naming convention), and palette-specific concepts
//! (`SwitchProvider`, Tab-cycle exclusion) would turn into stringly-
//! typed special cases keyed on `tag == "palette"`.
//!
//! The cost of keeping it a builtin (one special field on `UiState`,
//! one well-known `SURFACE_ID = "palette"`, one `AppCommand::OpenPalette`
//! arm, one special-case branch in `UiState::deliver_to_focused_surface`)
//! is localised and tagged. The cost of unification would be spread
//! across the `Plugin` trait contract. The current asymmetry is chosen.
//!
//! # Mechanics
//!
//! Concrete behaviour (items, filter, activation) lives on a
//! [`PaletteProvider`](provider::PaletteProvider). The palette swaps
//! providers when the user picks a "sub-mode" command (e.g. "Theme"
//! switches to [`ThemeProvider`](provider::ThemeProvider)). The
//! palette never touches `FocusManager`; focus transitions are
//! driven by `UiState::open_palette` emitting
//! `FocusEvent::Claimed(SURFACE_ID)` and by the delivery path
//! emitting `FocusEvent::Released(SURFACE_ID)` when `is_visible()`
//! flips to false.

pub mod panel;
pub mod provider;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app_command::{Effect, FocusSurface, SurfaceCtx};
use crate::color_palette::ThemeId;
use crate::keymap::KeyMap;
use crate::plugin::PluginRegistry;
use crate::theme::UiTheme;

use provider::{CommandProvider, PaletteAction, PaletteProvider};

/// `SurfaceId` for the palette in the focus system. Owned here so
/// the palette is the source of truth for its own identifier; other
/// modules import from this constant rather than hardcoding "palette".
pub const SURFACE_ID: &str = "palette";

pub struct CommandPalette {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) selected: usize,
    pub(super) provider: Option<Box<dyn PaletteProvider>>,
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            active: false,
            selected: 0,
            provider: None,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.active
    }

    /// Open the palette with the default [`CommandProvider`]. The host
    /// is responsible for taking palette focus afterwards — the palette
    /// does not touch `FocusManager` itself (mirrors the plugin rule).
    pub fn activate(&mut self, widgets: &PluginRegistry, keymap: &KeyMap, current_theme: ThemeId) {
        let provider = Box::new(CommandProvider::new(widgets, keymap, current_theme));
        self.open_with(provider);
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }

    fn open_with(&mut self, provider: Box<dyn PaletteProvider>) {
        self.query.clear();
        self.selected = 0;
        self.provider = Some(provider);
        self.active = true;
        if let Some(p) = self.provider.as_mut() {
            p.filter("");
        }
    }

    fn items_len(&self) -> usize {
        self.provider.as_ref().map(|p| p.items().len()).unwrap_or(0)
    }

    fn refilter(&mut self) {
        if let Some(p) = self.provider.as_mut() {
            p.filter(&self.query);
        }
        let n = self.items_len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }
}

/// The palette is modal: every key while it is focused is `Consumed`
/// (the responder chain stops here — never falls through to the
/// background). Item selection produces `Effect::Run(AppCommand)`.
impl FocusSurface for CommandPalette {
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        _ctx: SurfaceCtx,
    ) -> Effect {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let up = matches!(code, KeyCode::Up) || (ctrl && code == KeyCode::Char('p'));
        let down = matches!(code, KeyCode::Down) || (ctrl && code == KeyCode::Char('n'));

        match code {
            KeyCode::Esc => {
                self.active = false;
                Effect::Consumed
            }
            KeyCode::Enter => {
                let has_item = self.selected < self.items_len();
                self.active = false;
                if !has_item {
                    return Effect::Consumed;
                }
                let idx = self.selected;
                let action = self
                    .provider
                    .as_mut()
                    .map(|p| p.execute(idx))
                    .unwrap_or(PaletteAction::Close);
                match action {
                    PaletteAction::Close => Effect::Consumed,
                    PaletteAction::SwitchProvider(next) => {
                        // Provider-to-provider transition: stay active,
                        // reopen with the new provider. Host sees
                        // `is_visible()` still true, keeps focus.
                        self.open_with(next);
                        Effect::Consumed
                    }
                    PaletteAction::Run(cmd) => Effect::Run(cmd),
                }
            }
            _ if up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                Effect::Consumed
            }
            _ if down => {
                if self.selected + 1 < self.items_len() {
                    self.selected += 1;
                }
                Effect::Consumed
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
                Effect::Consumed
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.refilter();
                Effect::Consumed
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
                Effect::Consumed
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refilter();
                Effect::Consumed
            }
            // Modal: any other key is still consumed (don't fall
            // through to the background while the palette is up).
            _ => Effect::Consumed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_command::AppCommand;
    use crate::geo::LonLat;
    use crate::map::Action;
    use crate::ui::palette::provider::PaletteItem;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: SurfaceCtx = SurfaceCtx {
        center: LonLat { lon: 0.0, lat: 0.0 },
    };

    /// Minimal provider: lists the labels we give it, substring filter,
    /// `execute(idx)` returns `Run(AppCommand::Map(Action::None))`.
    struct FakeProvider {
        all: Vec<String>,
        filtered: Vec<usize>,
        items: Vec<PaletteItem>,
    }

    impl FakeProvider {
        fn new(labels: &[&str]) -> Self {
            let mut p = Self {
                all: labels.iter().map(|s| s.to_string()).collect(),
                filtered: Vec::new(),
                items: Vec::new(),
            };
            p.filter("");
            p
        }
    }

    impl PaletteProvider for FakeProvider {
        fn prompt(&self) -> &str {
            ":"
        }
        fn filter(&mut self, query: &str) {
            let q = query.to_lowercase();
            self.filtered = if q.is_empty() {
                (0..self.all.len()).collect()
            } else {
                let mut ranked: Vec<(usize, usize)> = self
                    .all
                    .iter()
                    .enumerate()
                    .filter_map(|(i, l)| l.to_lowercase().find(&q).map(|pos| (pos, i)))
                    .collect();
                ranked.sort_by_key(|&(pos, i)| (pos, i));
                ranked.into_iter().map(|(_, i)| i).collect()
            };
            self.items = self
                .filtered
                .iter()
                .map(|&i| PaletteItem {
                    label: self.all[i].clone(),
                    hint: String::new(),
                })
                .collect();
        }
        fn items(&self) -> &[PaletteItem] {
            &self.items
        }
        fn execute(&mut self, _idx: usize) -> PaletteAction {
            PaletteAction::Run(AppCommand::Map(Action::None))
        }
    }

    fn palette_with(labels: &[&str]) -> CommandPalette {
        let mut p = CommandPalette::new();
        p.open_with(Box::new(FakeProvider::new(labels)));
        p
    }

    fn filtered_labels(p: &CommandPalette) -> Vec<&str> {
        p.provider
            .as_ref()
            .unwrap()
            .items()
            .iter()
            .map(|i| i.label.as_str())
            .collect()
    }

    fn key(p: &mut CommandPalette, code: KeyCode, mods: KeyModifiers) -> Effect {
        p.handle_key(code, mods, CTX)
    }

    #[test]
    fn filter_empty_query_lists_all() {
        let p = palette_with(&["Zoom in", "Zoom out", "Quit"]);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Zoom out", "Quit"]);
    }

    #[test]
    fn filter_substring_case_insensitive() {
        let mut p = palette_with(&["Zoom in", "Zoom out", "Quit"]);
        key(&mut p, KeyCode::Char('Z'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Zoom out"]);
    }

    #[test]
    fn filter_earlier_match_ranks_first() {
        let mut p = palette_with(&["Zoom in", "Quit"]);
        // 'i' at pos 2 of "Quit" vs pos 5 of "Zoom in" → Quit first.
        key(&mut p, KeyCode::Char('i'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Quit", "Zoom in"]);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut p = palette_with(&["Zoom in", "Quit"]);
        key(&mut p, KeyCode::Char('z'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in"]);
        key(&mut p, KeyCode::Backspace, NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Quit"]);
    }

    #[test]
    fn down_up_stays_in_bounds() {
        let mut p = palette_with(&["A", "B", "C"]);
        key(&mut p, KeyCode::Down, NONE);
        key(&mut p, KeyCode::Down, NONE);
        key(&mut p, KeyCode::Down, NONE); // past end
        assert_eq!(p.selected, 2);
        key(&mut p, KeyCode::Up, NONE);
        key(&mut p, KeyCode::Up, NONE);
        key(&mut p, KeyCode::Up, NONE); // past top
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn enter_returns_run_with_selected_appcommand() {
        let mut p = palette_with(&["A", "B", "C"]);
        key(&mut p, KeyCode::Down, NONE);
        let effect = key(&mut p, KeyCode::Enter, NONE);
        assert_eq!(effect, Effect::Run(AppCommand::Map(Action::None)));
        assert!(!p.is_visible());
    }

    #[test]
    fn enter_with_empty_filter_closes_without_run() {
        let mut p = palette_with(&["Zoom in"]);
        key(&mut p, KeyCode::Char('x'), NONE);
        assert!(filtered_labels(&p).is_empty());
        let effect = key(&mut p, KeyCode::Enter, NONE);
        assert_eq!(effect, Effect::Consumed);
        assert!(!p.is_visible());
    }

    #[test]
    fn esc_closes() {
        let mut p = palette_with(&["A"]);
        key(&mut p, KeyCode::Esc, NONE);
        assert!(!p.is_visible());
    }

    #[test]
    fn ctrl_u_clears_query() {
        let mut p = palette_with(&["A"]);
        key(&mut p, KeyCode::Char('a'), NONE);
        key(&mut p, KeyCode::Char('b'), NONE);
        key(&mut p, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(p.query, "");
    }
}
