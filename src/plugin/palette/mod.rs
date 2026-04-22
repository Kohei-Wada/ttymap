//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! Under the compositor model, palette is an ephemeral
//! [`Component`]: pushed on `:`, popped on Esc/Enter/…. State
//! (`query`, `selected`, `provider`) is per-open and discarded on
//! pop — no `active` flag, no `seed` field on a long-lived struct.
//!
//! The list of non-palette palette entries is harvested from the
//! [`Registrar`](crate::compositor::Registrar) at composition time
//! (see [`register`]) and baked into a [`CommandSeed`] that the
//! activation closure clones (as an `Rc`) for each push.
//!
//! Provider sub-modes (Theme picker) are reached by the provider's
//! `execute` returning [`PaletteAction::SwitchProvider`]; the palette
//! swaps its internal `provider` field in place — no round-trip
//! through the compositor.

pub mod panel;
pub mod provider;

use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::color_palette::ThemeId;
use crate::compositor::{
    Activation, Component, Context, EventResult, Registrar,
};
use crate::keymap::KeyMap;
use crate::theme::UiTheme;

use provider::{CommandProvider, CommandSeed, PaletteAction, PaletteProvider};

pub struct PaletteComponent {
    pub(super) query: String,
    pub(super) selected: usize,
    pub(super) provider: Box<dyn PaletteProvider>,
}

impl PaletteComponent {
    fn with_provider(mut provider: Box<dyn PaletteProvider>) -> Self {
        provider.filter("");
        Self {
            query: String::new(),
            selected: 0,
            provider,
        }
    }

    pub fn new_default(seed: Rc<CommandSeed>, theme_id: ThemeId) -> Self {
        Self::with_provider(Box::new(CommandProvider::build(seed, theme_id)))
    }

    fn items_len(&self) -> usize {
        self.provider.items().len()
    }

    fn refilter(&mut self) {
        self.provider.filter(&self.query);
        let n = self.items_len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }
}

impl Component for PaletteComponent {
    fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> EventResult {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let up = matches!(event.code, KeyCode::Up) || (ctrl && event.code == KeyCode::Char('p'));
        let down = matches!(event.code, KeyCode::Down) || (ctrl && event.code == KeyCode::Char('n'));

        match event.code {
            KeyCode::Esc => EventResult::Close(Vec::new()),
            KeyCode::Enter => {
                let has_item = self.selected < self.items_len();
                if !has_item {
                    return EventResult::Close(Vec::new());
                }
                let idx = self.selected;
                let action = self.provider.execute(idx, ctx);
                match action {
                    PaletteAction::Close => EventResult::Close(Vec::new()),
                    PaletteAction::Run(msgs) => EventResult::Close(msgs),
                    PaletteAction::Push(component) => {
                        EventResult::CloseAndPush(component, Vec::new())
                    }
                    PaletteAction::SwitchProvider(next) => {
                        self.query.clear();
                        self.selected = 0;
                        self.provider = next;
                        self.provider.filter("");
                        EventResult::Consumed(Vec::new())
                    }
                }
            }
            _ if up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                EventResult::Consumed(Vec::new())
            }
            _ if down => {
                if self.selected + 1 < self.items_len() {
                    self.selected += 1;
                }
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.refilter();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refilter();
                EventResult::Consumed(Vec::new())
            }
            _ => EventResult::Consumed(Vec::new()),
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        panel::render_panel(self, f, area, theme);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }
}

/// Register the palette. Harvests all palette entries contributed by
/// other plugins (so call this **after** every other plugin's
/// `register`), bakes them into a [`CommandSeed`], and adds a single
/// `:` activation pointing at a fresh [`PaletteComponent`].
pub fn register(keymap: &KeyMap, r: &mut Registrar) {
    let plugin_entries = std::mem::take(&mut r.palette_entries);
    let seed = Rc::new(CommandSeed::build(keymap, plugin_entries));

    let seed_for_spawn = seed;
    r.add_activation(Activation {
        code: KeyCode::Char(':'),
        modifiers: KeyModifiers::NONE,
        spawn: Box::new(move |ctx: &Context| -> Box<dyn Component> {
            Box::new(PaletteComponent::new_default(
                seed_for_spawn.clone(),
                ctx.theme_id,
            ))
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::provider::{PaletteItem, PaletteProvider};
    use super::*;
    use crate::app::AppMsg;
    use crate::color_palette::ThemeId;
    use crate::geo::LonLat;
    use crate::map::Action;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: Context = Context {
        center: LonLat { lon: 0.0, lat: 0.0 },
        theme_id: ThemeId::Dark,
    };

    /// Minimal provider for testing: lists labels, substring filter,
    /// Enter returns `Run(Map(None))`.
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
        fn execute(&mut self, _idx: usize, _ctx: &Context) -> PaletteAction {
            PaletteAction::Run(vec![AppMsg::Map(Action::None)])
        }
    }

    fn palette_with(labels: &[&str]) -> PaletteComponent {
        PaletteComponent::with_provider(Box::new(FakeProvider::new(labels)))
    }

    fn filtered_labels(p: &PaletteComponent) -> Vec<&str> {
        p.provider
            .items()
            .iter()
            .map(|i| i.label.as_str())
            .collect()
    }

    fn key(p: &mut PaletteComponent, code: KeyCode, mods: KeyModifiers) -> EventResult {
        p.handle_event(KeyEvent::new(code, mods), &CTX)
    }

    fn expect_consumed(r: EventResult) {
        match r {
            EventResult::Consumed(msgs) => assert!(msgs.is_empty()),
            _ => panic!("expected Consumed"),
        }
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
        expect_consumed(key(&mut p, KeyCode::Down, NONE));
        expect_consumed(key(&mut p, KeyCode::Down, NONE));
        expect_consumed(key(&mut p, KeyCode::Down, NONE)); // past end
        assert_eq!(p.selected, 2);
        expect_consumed(key(&mut p, KeyCode::Up, NONE));
        expect_consumed(key(&mut p, KeyCode::Up, NONE));
        expect_consumed(key(&mut p, KeyCode::Up, NONE)); // past top
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn enter_returns_close_with_selected_msgs() {
        let mut p = palette_with(&["A", "B", "C"]);
        expect_consumed(key(&mut p, KeyCode::Down, NONE));
        let r = key(&mut p, KeyCode::Enter, NONE);
        match r {
            EventResult::Close(msgs) => {
                assert_eq!(msgs, vec![AppMsg::Map(Action::None)]);
            }
            _ => panic!("expected Close"),
        }
    }

    #[test]
    fn enter_with_empty_filter_closes_without_run() {
        let mut p = palette_with(&["Zoom in"]);
        key(&mut p, KeyCode::Char('x'), NONE);
        assert!(filtered_labels(&p).is_empty());
        let r = key(&mut p, KeyCode::Enter, NONE);
        match r {
            EventResult::Close(msgs) => assert!(msgs.is_empty()),
            _ => panic!("expected Close"),
        }
    }

    #[test]
    fn esc_closes() {
        let mut p = palette_with(&["A"]);
        let r = key(&mut p, KeyCode::Esc, NONE);
        match r {
            EventResult::Close(msgs) => assert!(msgs.is_empty()),
            _ => panic!("expected Close"),
        }
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
