//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! # Builtin plugin
//!
//! Palette is registered as a built-in [`Plugin`] alongside `search`,
//! `wiki`, `help`, `here`. It uses the same activation pipeline
//! (`activation_keys = [":"]` → `BackgroundResponder` →
//! `Effect::Open("palette")` → `FocusManager::open`) and lives in the
//! same registry. The asymmetry that previously justified treating it
//! as a special field on `FocusManager` was largely cosmetic: the
//! provider already walked `&PluginRegistry`, so the "plugins don't
//! see each other" invariant was already broken at the provider
//! layer. Folding palette in lets us delete a special branch in
//! `FocusManager::open`, the `palette` / `palette_mut` accessors, and
//! the `id == SURFACE_ID` check in `ui::draw`.
//!
//! What palette **does** keep that no other plugin needs:
//! - introspects sibling plugins at startup (via [`build`]) — same
//!   pattern as `HelpPlugin::build`. The captured snapshot is then
//!   combined per-open with the current theme id (read from
//!   [`SurfaceCtx::theme_id`] in `activate`) to seed the
//!   "Theme" sub-mode entry's "(current)" hint.
//!
//! # Mechanics
//!
//! Concrete behaviour (items, filter, activation) lives on a
//! [`PaletteProvider`](provider::PaletteProvider). The palette swaps
//! providers when the user picks a "sub-mode" command (e.g. "Theme"
//! switches to [`ThemeProvider`](provider::ThemeProvider)). The
//! palette never touches `FocusManager`; focus transitions are driven
//! by `FocusManager::open` (which the router invokes when a surface
//! returns `Effect::Open(SURFACE_ID)`) and the auto-release path
//! (`is_visible()` flipping to false after `handle_key`).

pub mod panel;
pub mod provider;

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::color_palette::ThemeId;
use crate::focus::{Effect, FocusSurface, SurfaceCtx};
use crate::keymap::KeyMap;
use crate::theme::UiTheme;

use super::Plugin;

use provider::{CommandProvider, CommandProviderSeed, PaletteAction, PaletteProvider};

/// `SurfaceId` for the palette in the focus system. Owned here so
/// the palette is the source of truth for its own identifier; other
/// modules import from this constant rather than hardcoding "palette".
pub const SURFACE_ID: &str = "palette";

pub struct CommandPalette {
    pub(super) query: String,
    pub(super) active: bool,
    pub(super) selected: usize,
    pub(super) provider: Option<Box<dyn PaletteProvider>>,
    /// Static portion of the default provider's entry list — captured
    /// at startup by [`build`]. Empty until `build` runs; activating
    /// before `build` shows only the dynamic Theme entry.
    seed: CommandProviderSeed,
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
            seed: CommandProviderSeed::default(),
        }
    }

    /// Capture the static portion of the default provider's entry
    /// list (map actions + every visible plugin's activation entry).
    /// Called once at startup after sibling plugins are constructed
    /// — same two-stage pattern as `HelpPlugin::build`. Re-running it
    /// is safe (overwrites the snapshot) but unnecessary because
    /// plugin metadata doesn't change at runtime.
    pub fn build(&mut self, keymap: &KeyMap, plugins: &[&dyn Plugin]) {
        self.seed = CommandProvider::snapshot(plugins, keymap);
    }

    fn open_default(&mut self, theme_id: ThemeId) {
        let provider = Box::new(CommandProvider::build(&self.seed, theme_id));
        self.open_with(provider);
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

impl Plugin for CommandPalette {
    fn tag(&self) -> &str {
        SURFACE_ID
    }

    /// Empty description — the palette opts out of being listed
    /// inside itself / inside the help overlay (it would be
    /// recursive / redundant).
    fn description(&self) -> &str {
        ""
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec![":"]
    }

    /// Open with the default [`CommandProvider`]. The palette is
    /// location-agnostic so it ignores `ctx.center`; it reads
    /// `ctx.theme_id` to seed the theme-picker entry's "(current)"
    /// hint, which is the only place the active theme leaks into
    /// the palette. The host (`FocusManager::open`) takes focus
    /// after this returns.
    fn activate(&mut self, ctx: SurfaceCtx) {
        self.open_default(ctx.theme_id);
    }

    /// Modal: any focus transfer (Tab, another plugin's activation
    /// key) closes the palette completely rather than leaving its
    /// popup orphaned on screen.
    fn deactivate(&mut self) {
        self.active = false;
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }
}

/// The palette is modal: every key while it is focused is `Consumed`
/// (the responder chain stops here — never falls through to the
/// background). Item selection produces `Effect::Run(Vec<AppMsg>)`.
impl FocusSurface for CommandPalette {
    fn is_visible(&self) -> bool {
        self.active
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers, _ctx: SurfaceCtx) -> Effect {
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
                    PaletteAction::Run(msgs) => Effect::Run(msgs),
                    PaletteAction::Open(id) => Effect::Open(id),
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
    use crate::app::AppMsg;
    use crate::geo::LonLat;
    use crate::map::Action;
    use crate::plugin::palette::provider::PaletteItem;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: SurfaceCtx = SurfaceCtx {
        center: LonLat { lon: 0.0, lat: 0.0 },
        theme_id: ThemeId::Dark,
    };

    /// Minimal provider: lists the labels we give it, substring filter,
    /// `execute(idx)` returns `Run(vec![AppMsg::Map(Action::None)])`.
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
            PaletteAction::Run(vec![AppMsg::Map(Action::None)])
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
    fn enter_returns_run_with_selected_msgs() {
        let mut p = palette_with(&["A", "B", "C"]);
        key(&mut p, KeyCode::Down, NONE);
        let effect = key(&mut p, KeyCode::Enter, NONE);
        assert_eq!(effect, Effect::Run(vec![AppMsg::Map(Action::None)]));
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
