//! Compositor — stack-based focus/modal system (helix-inspired).
//!
//! Replaces the `FocusManager` + `FocusSurface` + `Plugin` trio with a
//! single primitive: a stack of [`Component`]s. The top of the stack
//! holds focus; push on activation, pop on [`EventResult::Close`].
//! Object lifetime *is* the visibility lifecycle, so plugins never
//! have to maintain a separate `is_visible` / `activate` / `deactivate`
//! contract — fresh instances on every push, dropped on every pop.
//!
//! World-space map overlays (wiki markers etc.) live on
//! [`Component::paint_on_map`] — called for every component on the
//! stack. Tying map-side rendering to stack presence means markers
//! appear when the panel opens and disappear when it closes, in
//! step, without a second "is this paint active?" flag to keep in
//! sync.
//!
//! Plugin self-registration goes through [`Registrar`]: each plugin
//! module exposes
//!
//! ```ignore
//! pub fn register(config: &Config, r: &mut Registrar)
//! ```
//!
//! and constructs its own state + closures internally. `App` takes a
//! finished `Registrar` and never names a concrete plugin type — the
//! composition root (today `main.rs` / a dedicated plugins module) is
//! the one place that imports each plugin.

pub mod base;

pub use base::BaseLayer;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::painter::MapPainter;
use crate::theme::UiTheme;

// ── Framework-reserved keys ────────────────────────────────────────

/// Keys the compositor handles globally, without consulting any
/// component. Currently: `Tab` → forward cycle, `Shift-Tab` /
/// `BackTab` → backward cycle.
///
/// Intercepting here — rather than at `BaseLayer` — means no
/// component on the stack can accidentally absorb Tab. Focus cycling
/// is a property of the framework, not of any plugin's correctness.
fn intercept_focus_key(event: KeyEvent) -> Option<AppMsg> {
    if event.code == KeyCode::Tab && event.modifiers == KeyModifiers::NONE {
        return Some(AppMsg::CycleFocus(true));
    }
    if event.code == KeyCode::BackTab
        || (event.code == KeyCode::Tab && event.modifiers.contains(KeyModifiers::SHIFT))
    {
        return Some(AppMsg::CycleFocus(false));
    }
    None
}

// ── Component + event routing ──────────────────────────────────────

/// Outcome of delivering an event to a [`Component`].
///
/// - `Ignored`: the component is not interested; compositor tries the
///   next layer down. If nothing claims it, the event is discarded.
/// - `Consumed(msgs)`: absorbed, may emit messages, stack unchanged.
/// - `Close(msgs)`: absorbed + pop me. Messages dispatch before the
///   pop (so e.g. a `Jump` fires before the modal disappears).
/// - `Push(component, msgs)`: absorbed + push a new component on top
///   of me. Used by the bottom-layer keymap component to open modals
///   on activation keys without knowing about the compositor.
/// - `CloseAndPush(component, msgs)`: pop me and push `component`
///   next. Used by the palette: selecting a Spawn-kind entry closes
///   the palette and opens the target component.
pub enum EventResult {
    Ignored,
    Consumed(Vec<AppMsg>),
    Close(Vec<AppMsg>),
    Push(Box<dyn Component>, Vec<AppMsg>),
    CloseAndPush(Box<dyn Component>, Vec<AppMsg>),
}

/// Read-only snapshot of app-level context a component may need
/// during key handling. Equivalent to today's `SurfaceCtx`.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub center: LonLat,
    pub theme_id: ThemeId,
}

/// A focus-capable UI entity. Pushed on activation, popped on close.
/// No `is_visible` / `activate` / `deactivate` contract — existence on
/// the stack is the visibility lifecycle.
pub trait Component {
    /// Stable identifier for **deduplication**. When a new component
    /// with a matching tag is about to be pushed (`EventResult::Push`
    /// or `EventResult::CloseAndPush`), the compositor instead shifts
    /// focus to the existing instance and discards the newcomer.
    /// Prevents scenarios like "`i` pushes wiki1; Tab to base; `i`
    /// again pushes wiki2" from creating duplicate panels.
    ///
    /// Returning `None` opts out of dedup (every push is a fresh
    /// instance). The [`BaseLayer`] returns `None` because it's
    /// never the target of a push.
    fn tag(&self) -> Option<&'static str> {
        None
    }

    /// Handle a single key event. Return `Ignored` to let lower
    /// layers see it, `Consumed(msgs)` to absorb, `Close(msgs)` to
    /// absorb and pop, or `Push(c, msgs)` to absorb and push `c` on
    /// top.
    fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> EventResult;

    /// Paint this component into `area`. Called once per frame while
    /// on the stack; compositor renders bottom-to-top.
    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme);

    /// Paint world-space primitives on the map via [`MapPainter`].
    /// Called every frame while on the stack, before `render`. Default
    /// no-op for components with no map presence (search, palette,
    /// help). Wiki uses this for article markers — because it's gated
    /// on stack presence, the markers naturally disappear when the
    /// panel is popped.
    fn paint_on_map(&self, _p: &mut MapPainter<'_>) {}

    /// Advance async work and surface new messages. Called every tick
    /// on every component on the stack. Replaces `Plugin::poll()` +
    /// `Plugin::pending_msgs()` — one hook instead of two.
    fn poll(&mut self) -> Vec<AppMsg> {
        Vec::new()
    }

    /// Footer hints shown while this component is on top.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

// ── Compositor stack ───────────────────────────────────────────────

/// Stack of components + a separate **focused index** decoupled from
/// stack position. Replaces `FocusManager` and its `Focus::{Background,
/// Modal}` state machine.
///
/// The stack stores components in render order (bottom-up). The
/// [`BaseLayer`] sits at index 0 and never moves. The focused index
/// tracks which component key events target first — that can be the
/// [`BaseLayer`] even while modals are rendered above it, which is
/// how the old `Focus::Background` state maps into this design.
pub struct Compositor {
    stack: Vec<Box<dyn Component>>,
    /// Index of the component that receives key events first.
    /// Invariant: `focused_idx < stack.len()` whenever the stack is
    /// non-empty. After every push, this becomes the new top; after
    /// a close/pop it is clamped down to the new last index.
    focused_idx: usize,
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            focused_idx: 0,
        }
    }

    pub fn push(&mut self, c: Box<dyn Component>) {
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    fn clamp_focus_after_shrink(&mut self) {
        if !self.stack.is_empty() && self.focused_idx >= self.stack.len() {
            self.focused_idx = self.stack.len() - 1;
        }
    }

    /// Deliver a key event to the focused component first; if it
    /// returns [`EventResult::Ignored`] and the focus isn't already
    /// on the [`BaseLayer`], re-deliver to the base layer. This
    /// two-step routing restores the old "non-modal plugin passes
    /// unknown keys through to the keymap" semantic under the
    /// compositor model.
    ///
    /// `Tab` / `Shift-Tab` / `BackTab` are **framework-reserved**
    /// and never reach any component — focus cycling is a property
    /// of the framework, not of any individual plugin.
    pub fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> Vec<AppMsg> {
        if let Some(msg) = intercept_focus_key(event) {
            return vec![msg];
        }
        if self.stack.is_empty() {
            return Vec::new();
        }
        let focused = self.focused_idx;
        let first = self.stack[focused].handle_event(event, ctx);
        match first {
            EventResult::Ignored if focused != 0 => {
                // Non-modal fall-through: re-deliver to BaseLayer.
                let second = self.stack[0].handle_event(event, ctx);
                self.apply_event_result(0, second)
            }
            EventResult::Ignored => Vec::new(),
            result => self.apply_event_result(focused, result),
        }
    }

    fn apply_event_result(&mut self, idx: usize, result: EventResult) -> Vec<AppMsg> {
        match result {
            EventResult::Ignored => Vec::new(),
            EventResult::Consumed(msgs) => msgs,
            EventResult::Close(msgs) => {
                self.stack.remove(idx);
                self.clamp_focus_after_shrink();
                msgs
            }
            EventResult::Push(new_component, msgs) => {
                self.push_or_focus(new_component);
                msgs
            }
            EventResult::CloseAndPush(new_component, msgs) => {
                self.stack.remove(idx);
                self.clamp_focus_after_shrink();
                self.push_or_focus(new_component);
                msgs
            }
        }
    }

    /// Push `c` on top **unless** a component with the same
    /// [`Component::tag`] is already on the stack — in that case
    /// focus jumps to the existing instance and `c` is dropped. This
    /// makes repeated activation keys (e.g. `i` pressed twice with
    /// focus on the base between presses) idempotent instead of
    /// stacking duplicate panels.
    fn push_or_focus(&mut self, c: Box<dyn Component>) {
        if let Some(tag) = c.tag()
            && let Some(existing) = self.stack.iter().position(|s| s.tag() == Some(tag))
        {
            self.focused_idx = existing;
            return;
        }
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    /// Poll every component; messages appended in stack order.
    pub fn poll(&mut self) -> Vec<AppMsg> {
        let mut out = Vec::new();
        for c in self.stack.iter_mut() {
            out.extend(c.poll());
        }
        out
    }

    /// Render bottom-up so later pushes draw on top.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme) {
        for c in self.stack.iter() {
            c.render(f, area, theme);
        }
    }

    /// Walk every component on the stack and let it paint world-space
    /// primitives through the supplied [`MapPainter`]. Drawn before
    /// `render` so modal popups sit on top of any map markers.
    pub fn paint_on_map(&self, p: &mut MapPainter<'_>) {
        for c in self.stack.iter() {
            c.paint_on_map(p);
        }
    }

    /// Footer hints from the currently focused component.
    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.stack
            .get(self.focused_idx)
            .map(|c| c.footer_hints())
            .unwrap_or_default()
    }

    /// Rotate `focused_idx` through all components (including the
    /// BaseLayer). Forward moves up the stack then wraps to 0;
    /// backward moves down then wraps to top. The stack itself is
    /// unchanged — only which component receives keys first.
    ///
    /// This restores the old `Focus::Background` behaviour: with a
    /// single modal on top, Tab toggles focus between the modal and
    /// the base layer. With multiple modals, Tab walks through all
    /// of them and the base layer in turn.
    ///
    /// No-op when the stack has one element or fewer — nothing to
    /// cycle to.
    pub fn cycle(&mut self, forward: bool) {
        let len = self.stack.len();
        if len <= 1 {
            return;
        }
        self.focused_idx = if forward {
            (self.focused_idx + 1) % len
        } else {
            (self.focused_idx + len - 1) % len
        };
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Async tasks (headless plugins) ─────────────────────────────────

/// Headless async job — the shape `here` (geoip lookup → `Jump`)
/// needs. Polled every tick by `App`; may emit messages when
/// background work completes. No UI, no focus, no component.
pub trait Task {
    fn poll(&mut self) -> Vec<AppMsg>;
}

// ── Plugin self-registration (App is plugin-agnostic) ──────────────

/// Factory closure producing a fresh [`Component`] when the user
/// activates the corresponding surface. Receives a [`Context`]
/// snapshot so plugins that read app-level state at activation time
/// (e.g. palette seeds its "(current)" theme hint from `theme_id`)
/// can do so without a separate lifecycle hook.
pub type SpawnComponent = Box<dyn Fn(&Context) -> Box<dyn Component>>;

/// Closure that kicks off a headless action (typically: start an
/// async `Task` or emit one-shot `AppMsg`s) when a palette entry is
/// selected. Receives `Context` for the same reason as
/// [`SpawnComponent`].
pub type RunAction = Box<dyn Fn(&Context) -> Vec<AppMsg>>;

/// One activation entry — "when this key is pressed while nothing
/// modal is above the bottom layer, invoke `spawn` and push the
/// result".
pub struct Activation {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub spawn: SpawnComponent,
}

/// What a palette entry does when selected.
pub enum PaletteKind {
    /// Push a new component (search, wiki, palette sub-mode ...).
    Spawn(SpawnComponent),
    /// Fire-and-forget action (here's "jump to current location").
    Run(RunAction),
}

/// Palette entry description owned by the registrar.
pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    pub kind: PaletteKind,
}

/// Collector passed to each plugin's `register` function. Every
/// channel is optional — headless plugins add only a task + palette
/// entry; visual plugins add an activation + palette entry; wiki's
/// map markers live on the component itself (via
/// [`Component::paint_on_map`]) so they flow through activations,
/// not a separate registrar field.
#[derive(Default)]
pub struct Registrar {
    pub activations: Vec<Activation>,
    pub palette_entries: Vec<PaletteEntry>,
    pub tasks: Vec<Box<dyn Task>>,
}

impl Registrar {
    pub fn add_activation(&mut self, a: Activation) {
        self.activations.push(a);
    }
    pub fn add_palette_entry(&mut self, e: PaletteEntry) {
        self.palette_entries.push(e);
    }
    pub fn add_task(&mut self, t: Box<dyn Task>) {
        self.tasks.push(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test component that identifies itself via both
    /// `tag` and `footer_hints`. Used to verify cycle / push /
    /// dedup.
    struct TagComponent(&'static str);

    impl Component for TagComponent {
        fn tag(&self) -> Option<&'static str> {
            Some(self.0)
        }
        fn handle_event(&mut self, _: KeyEvent, _: &Context) -> EventResult {
            EventResult::Consumed(Vec::new())
        }
        fn render(&self, _: &mut Frame, _: Rect, _: &UiTheme) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![(self.0, "")]
        }
    }

    fn focused_tag(c: &Compositor) -> &'static str {
        c.footer_hints()
            .first()
            .map(|(k, _)| *k)
            .unwrap_or("<empty>")
    }

    fn make_with(tags: &[&'static str]) -> Compositor {
        let mut c = Compositor::new();
        for t in tags {
            c.push(Box::new(TagComponent(t)));
        }
        c
    }

    #[test]
    fn cycle_no_op_with_base_only() {
        let mut c = make_with(&["base"]);
        c.cycle(true);
        c.cycle(false);
        assert_eq!(focused_tag(&c), "base");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn cycle_toggles_focus_with_single_modal() {
        // [base, m] — m on top and focused by default. Tab toggles
        // focus to base (the old `Focus::Background` behaviour);
        // stack order never changes, only `focused_idx` moves.
        let mut c = make_with(&["base", "m"]);
        assert_eq!(focused_tag(&c), "m");
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");
        c.cycle(true);
        assert_eq!(focused_tag(&c), "m");
        // Backward wraps the other way in the two-element case.
        c.cycle(false);
        assert_eq!(focused_tag(&c), "base");
    }

    #[test]
    fn cycle_forward_walks_all_components() {
        // [base, A, B] — B focused initially (last pushed).
        // Forward: B → base → A → B. Stack order never changes.
        let mut c = make_with(&["base", "A", "B"]);
        assert_eq!(focused_tag(&c), "B");
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");
        c.cycle(true);
        assert_eq!(focused_tag(&c), "A");
        c.cycle(true);
        assert_eq!(focused_tag(&c), "B");
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn cycle_backward_walks_all_components_reverse() {
        // [base, A, B] — B focused initially.
        // Backward: B → A → base → B.
        let mut c = make_with(&["base", "A", "B"]);
        c.cycle(false);
        assert_eq!(focused_tag(&c), "A");
        c.cycle(false);
        assert_eq!(focused_tag(&c), "base");
        c.cycle(false);
        assert_eq!(focused_tag(&c), "B");
    }

    /// Minimal component that identifies itself via its `tag` method
    /// and produces a Push on a specific key — used to exercise the
    /// tag-dedup path.
    struct TaggedSpawner {
        my_tag: &'static str,
        spawn_tag: &'static str,
        spawn_key: KeyCode,
    }

    impl Component for TaggedSpawner {
        fn tag(&self) -> Option<&'static str> {
            Some(self.my_tag)
        }
        fn handle_event(&mut self, event: KeyEvent, _ctx: &Context) -> EventResult {
            if event.code == self.spawn_key {
                let tag = self.spawn_tag;
                return EventResult::Push(Box::new(TagComponent(tag)), Vec::new());
            }
            EventResult::Consumed(Vec::new())
        }
        fn render(&self, _: &mut Frame, _: Rect, _: &UiTheme) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![(self.my_tag, "")]
        }
    }

    /// Pushing a component whose tag matches something already on
    /// the stack is idempotent: focus moves to the existing one,
    /// no duplicate entry. Verifies the fix for "`i` with base
    /// focused while wiki is already open spawns wiki2".
    #[test]
    fn push_with_existing_tag_focuses_existing() {
        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
        };

        let mut c = Compositor::new();
        // Base layer that spawns a "wiki" tagged component on 'i'.
        c.push(Box::new(TaggedSpawner {
            my_tag: "base",
            spawn_tag: "wiki",
            spawn_key: KeyCode::Char('i'),
        }));
        // Open wiki for the first time.
        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2);
        assert_eq!(focused_tag(&c), "wiki");

        // Tab back to base.
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");

        // Press `i` again — base would spawn a second wiki, but
        // dedup catches it: focus moves to the existing one, stack
        // stays length 2.
        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2, "no duplicate wiki in stack");
        assert_eq!(focused_tag(&c), "wiki", "focus moves to existing wiki");
    }

    /// Tab delivery is framework-level: even a component that
    /// consumes every key (the "bad plugin" case) can't block it.
    /// This is the structural guarantee that replaced the per-plugin
    /// "remember to Ignore Tab" contract.
    #[test]
    fn tab_is_intercepted_before_components() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        /// Consumes literally every event, including Tab.
        struct SwallowsAll;
        impl Component for SwallowsAll {
            fn handle_event(&mut self, _: KeyEvent, _: &Context) -> EventResult {
                EventResult::Consumed(vec![AppMsg::Map(crate::map::Action::None)])
            }
            fn render(&self, _: &mut Frame, _: Rect, _: &UiTheme) {}
        }

        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
        };

        let mut c = Compositor::new();
        c.push(Box::new(SwallowsAll));

        let msgs = c.handle_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &ctx);
        assert_eq!(msgs, vec![AppMsg::CycleFocus(true)]);

        let msgs = c.handle_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), &ctx);
        assert_eq!(msgs, vec![AppMsg::CycleFocus(false)]);
    }
}
