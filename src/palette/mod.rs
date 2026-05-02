//! Command palette — `:`-triggered popup, a **universal picker**.
//!
//! Under the compositor model, palette is an ephemeral
//! [`Component`]: pushed on `:`, popped on Esc/Enter/…. State
//! (`query`, `selected`, `provider`) is per-open and discarded on
//! pop — no `active` flag, no `seed` field on a long-lived struct.
//!
//! The list of non-palette palette entries is harvested from the
//! [`Registrar`](crate::compositor::Registrar) at composition time
//! (see [`install`]) and baked into a [`CommandSeed`] that the
//! activation closure clones (as an `Rc`) for each push.
//!
//! Provider sub-modes (Theme picker) are reached by the provider's
//! `execute` returning [`PaletteAction::SwitchProvider`]; the palette
//! swaps its internal `provider` field in place — no round-trip
//! through the compositor.

pub mod panel;
pub mod provider;

use std::rc::Rc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::compositor::window::{RenderWindow, Window};
use crate::compositor::{Activation, Component, Context, Registrar};
use crate::keymap::KeyMap;
use crate::theme::ThemeId;

use provider::{CommandProvider, CommandSeed, PaletteAction, PaletteProvider, SubmitMode};

pub struct PaletteComponent {
    pub(super) query: String,
    pub(super) selected: usize,
    pub(super) provider: Box<dyn PaletteProvider>,
    /// Set when the query mutates under a debounced provider; cleared
    /// once we forward the query via `provider.filter`. The `Instant`
    /// records the most recent keystroke.
    pub(super) pending_since: Option<Instant>,
}

impl PaletteComponent {
    pub fn with_provider(mut provider: Box<dyn PaletteProvider>) -> Self {
        provider.filter("");
        Self {
            query: String::new(),
            selected: 0,
            provider,
            pending_since: None,
        }
    }

    pub fn new_default(seed: Rc<CommandSeed>, theme_id: ThemeId) -> Self {
        Self::with_provider(Box::new(CommandProvider::build(seed, theme_id)))
    }

    fn items_len(&self) -> usize {
        self.provider.items().len()
    }

    /// Whether the palette is showing a "..." loading indicator —
    /// either the provider is awaiting a result, or the query has
    /// changed under a debounced provider but we haven't dispatched
    /// `filter()` yet.
    pub(super) fn is_loading(&self) -> bool {
        self.provider.is_loading() || self.pending_since.is_some()
    }

    fn refilter(&mut self) {
        self.provider.filter(&self.query);
        self.pending_since = None;
        let n = self.items_len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }

    /// React to a query mutation. Sync providers refilter immediately;
    /// debounced ones defer — `poll` flushes once the timer elapses.
    /// `OnEnter` providers buffer silently; only Enter triggers them.
    fn on_query_changed(&mut self) {
        match self.provider.submit_mode() {
            SubmitMode::OnEachKey => self.refilter(),
            SubmitMode::Debounced(_) => self.pending_since = Some(Instant::now()),
            SubmitMode::OnEnter => {}
        }
    }

    /// Translate a [`PaletteAction`] into `win.*` ops. Shared by every
    /// trigger that goes through the provider — `execute` (Enter on
    /// item) and `cancel` (Esc, Enter on empty) both funnel here so
    /// the close-path semantics live in one place.
    fn apply_action(&mut self, action: PaletteAction, win: &mut Window) {
        match action {
            PaletteAction::Close => win.close(),
            PaletteAction::Run(msgs) => {
                for m in msgs {
                    win.emit(m);
                }
                win.close();
            }
            PaletteAction::Push(component) => {
                // The compositor applies `close` before `open` from
                // the same WindowOps, so the palette is out of the
                // way before the new component lands.
                win.close();
                win.open(component);
            }
            PaletteAction::SwitchProvider(next) => {
                self.query.clear();
                self.selected = 0;
                self.provider = next;
                self.provider.filter("");
            }
        }
    }
}

impl Component for PaletteComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let up = matches!(event.code, KeyCode::Up) || (ctrl && event.code == KeyCode::Char('p'));
        let down =
            matches!(event.code, KeyCode::Down) || (ctrl && event.code == KeyCode::Char('n'));

        match event.code {
            KeyCode::Esc => {
                let action = self.provider.cancel();
                self.apply_action(action, win);
            }
            KeyCode::Enter => {
                let has_item = self.selected < self.items_len();
                if has_item {
                    let idx = self.selected;
                    let action = self.provider.execute(idx, win.ctx());
                    self.apply_action(action, win);
                } else if matches!(self.provider.submit_mode(), SubmitMode::OnEnter) {
                    // OnEnter providers treat Enter on an empty list
                    // as "submit the current query" — the palette
                    // stays open while the provider runs `filter`
                    // (which typically kicks off an async fetch).
                    self.refilter();
                } else {
                    let action = self.provider.cancel();
                    self.apply_action(action, win);
                }
            }
            _ if up => {
                let n = self.items_len();
                if n > 0 {
                    self.selected = if self.selected == 0 {
                        n - 1
                    } else {
                        self.selected - 1
                    };
                }
            }
            _ if down => {
                let n = self.items_len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.on_query_changed();
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                self.on_query_changed();
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.on_query_changed();
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.on_query_changed();
            }
            _ => {}
        }
    }

    fn render(&self, win: &mut RenderWindow) {
        panel::render_panel(self, win);
    }

    fn poll(&mut self, _win: &mut Window) {
        // Debounced providers: forward the query once the keystroke
        // burst has been quiet for `interval`. No-op for OnEachKey
        // because `pending_since` is never set in that mode.
        if let (Some(t), SubmitMode::Debounced(interval)) =
            (self.pending_since, self.provider.submit_mode())
            && t.elapsed() >= interval
        {
            self.refilter();
        }
        self.provider.poll();
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("↑↓", "select"), ("Enter", "run"), ("Esc", "cancel")]
    }

    fn name(&self) -> &'static str {
        "palette"
    }
}

/// Install the palette as a built-in. Unlike a plugin's `register`,
/// this is a **sink**: it drains every palette entry contributed by
/// earlier `plugin::*::register` calls via `std::mem::take`, bakes
/// them into a [`CommandSeed`], and adds a single `:` activation
/// pointing at a fresh [`PaletteComponent`]. Must be called **after**
/// every other plugin's `register`.
pub fn install(keymap: &KeyMap, r: &mut Registrar) {
    let plugin_entries = std::mem::take(&mut r.palette_entries);
    let seed = Rc::new(CommandSeed::build(keymap, plugin_entries));

    let seed_for_spawn = seed;
    r.add_activation(Activation {
        code: KeyCode::Char(':'),
        modifiers: KeyModifiers::NONE,
        spawn: Box::new(move |ctx: &Context| -> Option<Box<dyn Component>> {
            Some(Box::new(PaletteComponent::new_default(
                seed_for_spawn.clone(),
                ctx.theme_id,
            )))
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::provider::{PaletteItem, PaletteProvider};
    use super::*;
    use crate::app::AppMsg;
    use crate::map::Action;
    use crate::theme::ThemeId;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: Context = Context {
        theme_id: ThemeId::Dark,
        cursor: None,
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

    use crate::compositor::window::WindowOps;

    fn dispatch(p: &mut PaletteComponent, code: KeyCode, mods: KeyModifiers) -> WindowOps {
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX);
            p.handle_event(KeyEvent::new(code, mods), &mut win);
        }
        ops
    }

    fn expect_consumed(ops: WindowOps) {
        assert!(!ops.close);
        assert!(ops.opens.is_empty());
        assert!(ops.msgs.is_empty());
    }

    #[test]
    fn filter_empty_query_lists_all() {
        let p = palette_with(&["Zoom in", "Zoom out", "Quit"]);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Zoom out", "Quit"]);
    }

    #[test]
    fn filter_substring_case_insensitive() {
        let mut p = palette_with(&["Zoom in", "Zoom out", "Quit"]);
        dispatch(&mut p, KeyCode::Char('Z'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Zoom out"]);
    }

    #[test]
    fn filter_earlier_match_ranks_first() {
        let mut p = palette_with(&["Zoom in", "Quit"]);
        dispatch(&mut p, KeyCode::Char('i'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Quit", "Zoom in"]);
    }

    #[test]
    fn backspace_widens_filter() {
        let mut p = palette_with(&["Zoom in", "Quit"]);
        dispatch(&mut p, KeyCode::Char('z'), NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in"]);
        dispatch(&mut p, KeyCode::Backspace, NONE);
        assert_eq!(filtered_labels(&p), vec!["Zoom in", "Quit"]);
    }

    #[test]
    fn down_wraps_at_bottom_up_wraps_at_top() {
        let mut p = palette_with(&["A", "B", "C"]);
        expect_consumed(dispatch(&mut p, KeyCode::Down, NONE));
        expect_consumed(dispatch(&mut p, KeyCode::Down, NONE));
        assert_eq!(p.selected, 2);
        expect_consumed(dispatch(&mut p, KeyCode::Down, NONE)); // bottom -> top
        assert_eq!(p.selected, 0);
        expect_consumed(dispatch(&mut p, KeyCode::Up, NONE)); // top -> bottom
        assert_eq!(p.selected, 2);
    }

    #[test]
    fn ctrl_p_at_top_wraps_without_typing() {
        let mut p = palette_with(&["A", "B", "C"]);
        expect_consumed(dispatch(&mut p, KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(p.selected, 2);
        assert_eq!(p.query, "");
    }

    #[test]
    fn ctrl_n_at_bottom_wraps_without_typing() {
        let mut p = palette_with(&["A", "B", "C"]);
        dispatch(&mut p, KeyCode::Down, NONE);
        dispatch(&mut p, KeyCode::Down, NONE);
        assert_eq!(p.selected, 2);
        expect_consumed(dispatch(&mut p, KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(p.selected, 0);
        assert_eq!(p.query, "");
    }

    #[test]
    fn up_down_on_empty_list_is_noop() {
        let mut p = palette_with(&["Zoom"]);
        dispatch(&mut p, KeyCode::Char('x'), NONE); // filter to empty
        assert!(filtered_labels(&p).is_empty());
        expect_consumed(dispatch(&mut p, KeyCode::Char('p'), KeyModifiers::CONTROL));
        expect_consumed(dispatch(&mut p, KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(p.query, "x");
    }

    #[test]
    fn enter_closes_with_selected_msgs() {
        let mut p = palette_with(&["A", "B", "C"]);
        expect_consumed(dispatch(&mut p, KeyCode::Down, NONE));
        let ops = dispatch(&mut p, KeyCode::Enter, NONE);
        assert!(ops.close);
        assert_eq!(ops.msgs, vec![AppMsg::Map(Action::None)]);
        assert!(ops.opens.is_empty());
    }

    #[test]
    fn enter_with_empty_filter_closes_without_run() {
        let mut p = palette_with(&["Zoom in"]);
        dispatch(&mut p, KeyCode::Char('x'), NONE);
        assert!(filtered_labels(&p).is_empty());
        let ops = dispatch(&mut p, KeyCode::Enter, NONE);
        assert!(ops.close);
        assert!(ops.msgs.is_empty());
    }

    #[test]
    fn esc_closes() {
        let mut p = palette_with(&["A"]);
        let ops = dispatch(&mut p, KeyCode::Esc, NONE);
        assert!(ops.close);
        assert!(ops.msgs.is_empty());
    }

    /// Provider that counts how many times `cancel` is invoked. Used
    /// to verify Esc and Enter-on-empty both funnel through the
    /// provider rather than short-circuiting straight to `win.close()`.
    struct CancelCountProvider {
        items: Vec<PaletteItem>,
        cancel_calls: std::rc::Rc<std::cell::Cell<u32>>,
    }

    impl PaletteProvider for CancelCountProvider {
        fn prompt(&self) -> &str {
            ":"
        }
        fn filter(&mut self, _q: &str) {}
        fn items(&self) -> &[PaletteItem] {
            &self.items
        }
        fn execute(&mut self, _idx: usize, _ctx: &Context) -> PaletteAction {
            PaletteAction::Close
        }
        fn cancel(&mut self) -> PaletteAction {
            self.cancel_calls.set(self.cancel_calls.get() + 1);
            PaletteAction::Close
        }
    }

    #[test]
    fn esc_calls_provider_cancel() {
        let cancels = std::rc::Rc::new(std::cell::Cell::new(0));
        let prov = CancelCountProvider {
            items: vec![PaletteItem {
                label: "A".into(),
                hint: String::new(),
            }],
            cancel_calls: cancels.clone(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        let ops = dispatch(&mut p, KeyCode::Esc, NONE);
        assert!(ops.close);
        assert_eq!(cancels.get(), 1);
    }

    #[test]
    fn enter_on_empty_calls_provider_cancel() {
        let cancels = std::rc::Rc::new(std::cell::Cell::new(0));
        let prov = CancelCountProvider {
            items: Vec::new(),
            cancel_calls: cancels.clone(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        let ops = dispatch(&mut p, KeyCode::Enter, NONE);
        assert!(ops.close);
        assert_eq!(cancels.get(), 1);
    }

    #[test]
    fn enter_on_item_does_not_call_cancel() {
        let cancels = std::rc::Rc::new(std::cell::Cell::new(0));
        let prov = CancelCountProvider {
            items: vec![PaletteItem {
                label: "A".into(),
                hint: String::new(),
            }],
            cancel_calls: cancels.clone(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        let ops = dispatch(&mut p, KeyCode::Enter, NONE);
        assert!(ops.close);
        assert_eq!(cancels.get(), 0);
    }

    #[test]
    fn ctrl_u_clears_query() {
        let mut p = palette_with(&["A"]);
        dispatch(&mut p, KeyCode::Char('a'), NONE);
        dispatch(&mut p, KeyCode::Char('b'), NONE);
        dispatch(&mut p, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(p.query, "");
    }

    /// Provider that records each `filter()` invocation and reports a
    /// debounced submit mode. Used to verify `PaletteComponent` defers
    /// dispatch until `poll()` finds the debounce window has elapsed.
    struct DebouncedProvider {
        interval: std::time::Duration,
        calls: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        items: Vec<PaletteItem>,
    }

    impl DebouncedProvider {
        fn new(
            interval: std::time::Duration,
        ) -> (Self, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
            let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            (
                Self {
                    interval,
                    calls: calls.clone(),
                    items: Vec::new(),
                },
                calls,
            )
        }
    }

    impl PaletteProvider for DebouncedProvider {
        fn prompt(&self) -> &str {
            "/"
        }
        fn filter(&mut self, query: &str) {
            self.calls.borrow_mut().push(query.to_string());
        }
        fn items(&self) -> &[PaletteItem] {
            &self.items
        }
        fn execute(&mut self, _idx: usize, _ctx: &Context) -> PaletteAction {
            PaletteAction::Close
        }
        fn submit_mode(&self) -> SubmitMode {
            SubmitMode::Debounced(self.interval)
        }
    }

    #[test]
    fn debounced_provider_does_not_filter_per_keystroke() {
        let (prov, calls) = DebouncedProvider::new(std::time::Duration::from_millis(100));
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        // Construction calls `filter("")` once.
        assert_eq!(*calls.borrow(), vec!["".to_string()]);
        dispatch(&mut p, KeyCode::Char('t'), NONE);
        dispatch(&mut p, KeyCode::Char('o'), NONE);
        dispatch(&mut p, KeyCode::Char('k'), NONE);
        // No additional filter calls — query is buffered.
        assert_eq!(*calls.borrow(), vec!["".to_string()]);
        assert!(p.pending_since.is_some());
    }

    #[test]
    fn debounced_provider_filters_after_interval_in_poll() {
        let (prov, calls) = DebouncedProvider::new(std::time::Duration::from_millis(0));
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        dispatch(&mut p, KeyCode::Char('t'), NONE);
        dispatch(&mut p, KeyCode::Char('o'), NONE);
        // pending; poll should flush since interval is 0.
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX);
            p.poll(&mut win);
        }
        assert_eq!(*calls.borrow(), vec!["".to_string(), "to".to_string()]);
        assert!(p.pending_since.is_none());
    }

    /// Provider in `OnEnter` mode that records `filter` calls. Used
    /// to verify Enter on empty triggers `filter` (not `cancel`) and
    /// query mutations buffer silently in this mode.
    struct OnEnterProvider {
        calls: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        items: Vec<PaletteItem>,
    }

    impl PaletteProvider for OnEnterProvider {
        fn prompt(&self) -> &str {
            "/"
        }
        fn filter(&mut self, query: &str) {
            self.calls.borrow_mut().push(query.to_string());
        }
        fn items(&self) -> &[PaletteItem] {
            &self.items
        }
        fn execute(&mut self, _idx: usize, _ctx: &Context) -> PaletteAction {
            PaletteAction::Close
        }
        fn submit_mode(&self) -> SubmitMode {
            SubmitMode::OnEnter
        }
    }

    #[test]
    fn on_enter_mode_buffers_query_silently() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let prov = OnEnterProvider {
            calls: calls.clone(),
            items: Vec::new(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        // Construction calls `filter("")`.
        assert_eq!(*calls.borrow(), vec!["".to_string()]);
        dispatch(&mut p, KeyCode::Char('t'), NONE);
        dispatch(&mut p, KeyCode::Char('o'), NONE);
        // Typing must not call filter; `pending_since` stays None
        // because OnEnter has no debounce timer to flush.
        assert_eq!(*calls.borrow(), vec!["".to_string()]);
        assert!(p.pending_since.is_none());
    }

    #[test]
    fn on_enter_mode_enter_on_empty_triggers_filter_keeps_open() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let prov = OnEnterProvider {
            calls: calls.clone(),
            items: Vec::new(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        dispatch(&mut p, KeyCode::Char('t'), NONE);
        dispatch(&mut p, KeyCode::Char('o'), NONE);
        let ops = dispatch(&mut p, KeyCode::Enter, NONE);
        assert!(!ops.close, "OnEnter Enter+empty must keep palette open");
        assert_eq!(*calls.borrow(), vec!["".to_string(), "to".to_string()]);
    }

    #[test]
    fn on_enter_mode_esc_still_closes() {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let prov = OnEnterProvider {
            calls: calls.clone(),
            items: Vec::new(),
        };
        let mut p = PaletteComponent::with_provider(Box::new(prov));
        dispatch(&mut p, KeyCode::Char('t'), NONE);
        let ops = dispatch(&mut p, KeyCode::Esc, NONE);
        assert!(ops.close);
    }

    #[test]
    fn on_each_key_provider_filters_synchronously() {
        let mut p = palette_with(&["A", "B"]);
        // Mutating the query should refilter immediately under the
        // default `OnEachKey` mode. Use an existing provider whose
        // filter we can observe via items.
        dispatch(&mut p, KeyCode::Char('a'), NONE);
        assert_eq!(filtered_labels(&p), vec!["A"]);
        assert!(p.pending_since.is_none());
    }
}
