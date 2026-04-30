//! Compositor — stack-based focus/modal system (helix-inspired).
//!
//! Replaces the `FocusManager` + `FocusSurface` + `Plugin` trio with a
//! single primitive: a stack of [`Component`]s. The top of the stack
//! holds focus; push on activation, pop when a component calls
//! `win.close()`.
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
pub mod layout;
pub mod map_api;
pub mod window;

pub use base::BaseLayer;
pub use map_api::MapApi;

use std::any::Any;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::geo::LonLat;
use crate::theme::ThemeId;
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

/// Read-only snapshot of app-level context a component may need
/// during a hook. Reached by the component through
/// [`Window::ctx`](window::Window::ctx).
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub center: LonLat,
    pub theme_id: ThemeId,
    /// Latest mouse cursor position in absolute terminal cells.
    /// `None` until the first mouse event arrives (or always, on
    /// terminals without mouse support). Project to a `LonLat` via
    /// [`MapApi::cursor_ll`](crate::compositor::MapApi::cursor_ll)
    /// at paint time.
    #[allow(dead_code)] // plugin-author API; the in-tree reader (info plugin) lands later
    pub cursor: Option<(u16, u16)>,
}

/// A focus-capable UI entity. Pushed on activation, popped on close.
/// No `is_visible` / `activate` / `deactivate` contract — existence on
/// the stack is the visibility lifecycle.
///
/// Component extends [`Any`] so the compositor can deduplicate pushes
/// by concrete type without each plugin having to declare a stable
/// tag. Pressing an activation key twice with the base focused in
/// between still produces one instance of the plugin on the stack,
/// because the framework notices the type already present.
///
/// The event-producing hooks ([`handle_event`](Self::handle_event)
/// and [`poll`](Self::poll)) receive a
/// [`&mut Window`](window::Window) and express intent through it
/// (`win.close()`, `win.open(c)`, `win.emit(msg)`, `win.ignore()`).
/// The framework applies those ops atomically after the hook
/// returns, so components cannot break stack / focus invariants
/// regardless of what order they call the methods.
pub trait Component: Any {
    /// Handle a single key event. Call `win.close()` / `open(c)` /
    /// `emit(msg)` / `ignore()` to express what should happen next.
    /// Silence (no `win.*` call) is implicit consumption — the
    /// event is treated as handled but with no state change.
    ///
    /// Default impl is `win.ignore()` — the non-modal "I don't bind
    /// any keys, pass through to the base layer" behaviour. Plugins
    /// that consume keys override this.
    fn handle_event(&mut self, _event: KeyEvent, win: &mut Window) {
        win.ignore();
    }

    /// Paint this component into `win.area()`. Called once per
    /// frame while on the stack; compositor renders bottom-to-top.
    /// `win` carries the ratatui frame, the component's allowed
    /// area, and the current theme — plugins read all three through
    /// `win` so theme does not thread through helper signatures.
    ///
    /// Default impl is no-op — for marker-only components that have
    /// no panel UI (just `paint_on_map`).
    fn render(&self, _win: &mut window::RenderWindow) {}

    /// Paint world-space primitives on the map via [`MapApi`].
    /// Called every frame while on the stack, before `render`. Default
    /// no-op for components with no map presence (search, palette,
    /// help). Wiki uses this for article markers — because it's gated
    /// on stack presence, the markers naturally disappear when the
    /// panel is popped.
    fn paint_on_map(&self, _p: &mut MapApi<'_>) {}

    /// Advance async work and surface new messages. Called every tick
    /// on every component on the stack. Use `win.emit(msg)` to
    /// dispatch app-level state changes when a future completes,
    /// and `win.close()` if the component should self-remove.
    fn poll(&mut self, _win: &mut Window) {}

    /// Overlay counterpart to [`poll`](Self::poll). Called every tick
    /// on every always-on overlay. The handle is narrower —
    /// [`OverlayWindow`] only exposes `ctx` and `emit`, because
    /// overlays don't live on the focusable stack and `close` /
    /// `open` would have nothing to act on. Default no-op for
    /// stack-only components that never register as overlays.
    fn poll_overlay(&mut self, _win: &mut window::OverlayWindow) {}

    /// Footer hints shown while this component is on top.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Short user-facing label shown in the footer when this
    /// component is focused — e.g. `"wiki"`, `"aircraft"`. Defaults
    /// to empty so the bottom layer (or any unlabelled component)
    /// renders no chrome. Plugins return a fixed string token.
    fn name(&self) -> &'static str {
        ""
    }

    /// Stable identity used by the compositor when deduplicating
    /// stack pushes (`push_or_focus`, `push_or_toggle`). `None`
    /// keeps today's `Any::type_id` semantics — concrete-type dedup,
    /// which is the right shape for Rust-side components where the
    /// type itself *is* the identity.
    ///
    /// Adapters that share a single Rust type across many distinct
    /// plugin instances (notably [`LuaComponent`](crate::lua::LuaComponent),
    /// where every Lua-driven plugin is wrapped in the same struct)
    /// must override this to return a per-instance string. Without
    /// the override the compositor would treat all Lua plugins as
    /// the same kind and refuse to stack a second one on top of the
    /// first.
    fn dedup_tag(&self) -> Option<&str> {
        None
    }
}

use window::{OverlayWindow, Window, WindowOps};

/// Identity comparison for stack dedup. Two components are
/// considered "the same kind" when both supply a [`dedup_tag`](Component::dedup_tag)
/// and the tags match; otherwise we fall back to `Any::type_id`.
/// Mixed cases (one tagged, one untagged) compare via TypeId — a
/// Rust component and a Lua component never accidentally collide on
/// a shared name.
fn same_identity(a: &dyn Component, b: &dyn Component) -> bool {
    match (a.dedup_tag(), b.dedup_tag()) {
        (Some(x), Some(y)) => x == y,
        _ => Any::type_id(a) == Any::type_id(b),
    }
}

/// What to do when [`Compositor::push_or`] hits an existing
/// component with the same dedup identity. `Focus` is the activation-
/// key path (idempotent re-press refocuses); `Close` is the toggle-
/// palette path (re-toggle closes).
#[derive(Clone, Copy)]
enum OnCollision {
    Focus,
    Close,
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
    /// Always-on, non-focusable Components painted **after** the
    /// stack so chrome (info bar, scale, attribution) sits on top of
    /// any toggleable plugin's markers. Populated once at startup
    /// from `Registrar::overlays`; never receive key events and
    /// never participate in focus cycling.
    overlays: Vec<Box<dyn Component>>,
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
            overlays: Vec::new(),
            focused_idx: 0,
        }
    }

    pub fn push(&mut self, c: Box<dyn Component>) {
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    /// Install an always-on overlay. Called once at app init from
    /// the registrar; the component stays for the app's lifetime,
    /// paints after every regular stack component, and never
    /// receives key events.
    pub fn add_overlay(&mut self, c: Box<dyn Component>) {
        self.overlays.push(c);
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    fn clamp_focus_after_shrink(&mut self) {
        if !self.stack.is_empty() && self.focused_idx >= self.stack.len() {
            self.focused_idx = self.stack.len() - 1;
        }
    }

    /// Deliver a key event to the focused component first; if the
    /// component called only `win.ignore()` and focus isn't already
    /// on the [`BaseLayer`], re-deliver to the base layer. This
    /// two-step routing restores the old "non-modal plugin passes
    /// unknown keys through to the keymap" semantic.
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
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, ctx);
            self.stack[focused].handle_event(event, &mut win);
        }
        // Fall-through: only when the hook queued nothing *and*
        // explicitly called `ignore()`, and the focus isn't already
        // on the base layer.
        if ops.is_ignorable_noop() && ops.ignored && focused != 0 {
            let mut ops = WindowOps::default();
            {
                let mut win = Window::new(&mut ops, ctx);
                self.stack[0].handle_event(event, &mut win);
            }
            return self.apply_ops(0, ops);
        }
        self.apply_ops(focused, ops)
    }

    /// Drain a [`WindowOps`] queue in the documented order:
    /// `close` → `opens` (dedup → refocus) → `toggles` (dedup →
    /// close) → return `msgs` for the caller to dispatch. See
    /// [`window`] module docs.
    fn apply_ops(&mut self, idx: usize, ops: WindowOps) -> Vec<AppMsg> {
        if ops.close {
            self.stack.remove(idx);
            self.clamp_focus_after_shrink();
        }
        for c in ops.opens {
            self.push_or(c, OnCollision::Focus);
        }
        for c in ops.toggles {
            self.push_or(c, OnCollision::Close);
        }
        ops.msgs
    }

    /// Push `c` on top, or — when a component matching its dedup
    /// identity is already on the stack — fold the new push into
    /// the existing instance per `on_collision`:
    ///
    /// - `OnCollision::Focus` (used by `opens`): refocus the
    ///   existing one, drop `c`. Makes activation keys idempotent
    ///   so e.g. pressing `i` twice with focus on the base between
    ///   presses doesn't stack a second wiki panel.
    /// - `OnCollision::Close` (used by `toggles`): remove the
    ///   existing one, drop `c`. The "Toggle X" palette semantic.
    ///
    /// Identity defers to [`Component::dedup_tag`] (compared as
    /// strings when both sides supply one) and falls back to
    /// `Any::type_id` otherwise — Rust-side components keep
    /// concrete-type dedup for free; adapters like `LuaComponent`
    /// opt in by returning their plugin-supplied name.
    fn push_or(&mut self, c: Box<dyn Component>, on_collision: OnCollision) {
        if let Some(existing) = self.stack.iter().position(|s| same_identity(&**s, &*c)) {
            match on_collision {
                OnCollision::Focus => self.focused_idx = existing,
                OnCollision::Close => {
                    self.stack.remove(existing);
                    self.clamp_focus_after_shrink();
                }
            }
            return;
        }
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    /// Poll every component; drain all queued `win.emit(...)` /
    /// `win.close()` / `win.open(...)` ops and apply them in the
    /// same way [`handle_event`](Self::handle_event) does. Always-on
    /// overlays poll through the narrower [`Component::poll_overlay`]
    /// hook — they may have async work (geocoding, throttle ticks)
    /// but only `emit` is exposed because overlays don't live on the
    /// focusable stack.
    pub fn poll(&mut self, ctx: &Context) -> Vec<AppMsg> {
        // Walk in reverse so closing a component doesn't disturb
        // indices of later ones. Collect ops per index first; apply
        // after the borrow of `stack` is released.
        let mut all_msgs: Vec<AppMsg> = Vec::new();
        let len = self.stack.len();
        for i in (0..len).rev() {
            let mut ops = WindowOps::default();
            {
                let mut win = Window::new(&mut ops, ctx);
                self.stack[i].poll(&mut win);
            }
            let msgs = self.apply_ops(i, ops);
            all_msgs.extend(msgs);
        }
        for c in self.overlays.iter_mut() {
            let mut win = OverlayWindow::new(&mut all_msgs, ctx);
            c.poll_overlay(&mut win);
        }
        all_msgs
    }

    /// Render bottom-up so later pushes draw on top — with one
    /// twist: the **focused** component renders last (on top of
    /// everything else), regardless of where it sits in the stack.
    /// This lets multiple panels overlap freely; whichever the user
    /// is currently driving with the keyboard pops to the front.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme, ctx: &Context) {
        for (i, c) in self.stack.iter().enumerate() {
            if i == self.focused_idx {
                continue;
            }
            let mut win = window::RenderWindow::new(f, area, theme, ctx);
            c.render(&mut win);
        }
        if let Some(c) = self.stack.get(self.focused_idx) {
            let mut win = window::RenderWindow::new(f, area, theme, ctx);
            c.render(&mut win);
        }
    }

    /// Walk every component and let it paint world-space primitives
    /// through the supplied [`MapApi`]. Stack first (markers from
    /// toggleable plugins), then always-on overlays (chrome on top
    /// of those markers). Drawn before `render` so modal popups sit
    /// on top of everything.
    pub fn paint_on_map(&self, p: &mut MapApi<'_>) {
        for c in self.stack.iter() {
            c.paint_on_map(p);
        }
        for c in self.overlays.iter() {
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

    /// Name of the currently focused component (empty when the focus
    /// is on the base layer or any component that opted not to label
    /// itself). Surfaced in the footer so the user can tell which
    /// plugin is consuming their keystrokes when modals stack.
    pub fn focused_name(&self) -> &'static str {
        self.stack
            .get(self.focused_idx)
            .map(|c| c.name())
            .unwrap_or("")
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

// ── Plugin self-registration (App is plugin-agnostic) ──────────────

/// Factory closure producing a fresh [`Component`] when the user
/// activates the corresponding surface. Receives a [`Context`]
/// snapshot so plugins that read app-level state at activation time
/// (e.g. palette seeds its "(current)" theme hint from `theme_id`)
/// can do so without a separate lifecycle hook.
pub type SpawnComponent = Box<dyn Fn(&Context) -> Box<dyn Component>>;

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
    /// Push a new component. TypeId dedup refocuses an existing
    /// instance of the same concrete type (search, palette sub-mode, …).
    Spawn(SpawnComponent),
    /// Toggle semantics: same as [`Spawn`](Self::Spawn) on first
    /// selection, but closes the existing instance on re-selection.
    /// Used by palette labels of the form "Toggle X".
    Toggle(SpawnComponent),
}

/// Palette entry description owned by the registrar.
pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    /// Plugin's canonical short name (`module.name`). Used as the
    /// footer slug paired with `hint` (`[<hint> <name>]`).
    pub name: &'static str,
    pub kind: PaletteKind,
}

/// Collector passed to each plugin's `register` function. Plugins
/// add an activation, a palette entry, and / or an overlay; the
/// compositor stays agnostic of any specific plugin.
#[derive(Default)]
pub struct Registrar {
    pub activations: Vec<Activation>,
    pub palette_entries: Vec<PaletteEntry>,
    /// Always-on overlay factories — invoked once at app init and
    /// pushed into [`Compositor::overlays`]. Used for chrome that's
    /// always on screen (info bar, scale bar, attribution).
    pub overlays: Vec<SpawnComponent>,
}

impl Registrar {
    pub fn add_activation(&mut self, a: Activation) {
        self.activations.push(a);
    }
    pub fn add_palette_entry(&mut self, e: PaletteEntry) {
        self.palette_entries.push(e);
    }

    // ── Convenience builders ───────────────────────────────────────────────
    //
    // The methods below accept an `impl Component`-returning closure
    // and box twice internally so each plugin's `register` can drop
    // the `Box::new(move |...| -> Box<dyn Component> { Box::new(...) })`
    // syntactic noise. The struct-literal forms above stay for any
    // plugin that needs full control (e.g. building entries
    // dynamically).

    /// Bind a key to spawn a fresh component on press.
    pub fn bind<F, C>(&mut self, code: KeyCode, modifiers: KeyModifiers, factory: F)
    where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_activation(Activation {
            code,
            modifiers,
            spawn: box_component_factory(factory),
        });
    }

    /// Add a palette entry that toggles a component on/off — opens it
    /// on first selection, closes the existing instance on
    /// re-selection.
    pub fn add_toggle<F, C>(
        &mut self,
        label: impl Into<String>,
        hint: impl Into<String>,
        name: &'static str,
        factory: F,
    ) where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            name,
            kind: PaletteKind::Toggle(box_component_factory(factory)),
        });
    }

    /// Add a palette entry that spawns a fresh instance every time —
    /// no toggle dedup. Use when the component is meant to be rebuilt
    /// per open (search, palette sub-modes).
    pub fn add_spawn<F, C>(
        &mut self,
        label: impl Into<String>,
        hint: impl Into<String>,
        name: &'static str,
        factory: F,
    ) where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            name,
            kind: PaletteKind::Spawn(box_component_factory(factory)),
        });
    }

    /// Register an always-on overlay component. Pushed once at app
    /// init into [`Compositor::overlays`]; paints after every
    /// regular stack component, never receives key events. Use for
    /// chrome that's always on screen (info bar, scale, attribution).
    #[allow(dead_code)] // plugin-author API; in-tree consumers (info / scalebar / attribution plugins) land later
    pub fn add_overlay<F, C>(&mut self, factory: F)
    where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.overlays.push(box_component_factory(factory));
    }
}

/// Wrap an `impl Component`-returning closure in the double-Box that
/// the registrar's collections store. Lifts the `Box::new(move |ctx|
/// Box::new(factory(ctx)) as Box<dyn Component>)` boilerplate out of
/// every `add_*` method so the next builder doesn't have to remember
/// the exact dance.
fn box_component_factory<F, C>(factory: F) -> SpawnComponent
where
    F: Fn(&Context) -> C + 'static,
    C: Component + 'static,
{
    Box::new(move |ctx| Box::new(factory(ctx)) as Box<dyn Component>)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test component that identifies itself via
    /// `footer_hints`. Distinct string parameters are just labels;
    /// dedup in the compositor is by concrete type (`Any::type_id`),
    /// so two `TagComponent` instances are always considered the
    /// same kind regardless of the inner string.
    struct TagComponent(&'static str);

    impl Component for TagComponent {
        fn handle_event(&mut self, _: KeyEvent, _: &mut Window) {}
        fn render(&self, _: &mut window::RenderWindow) {}
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

    /// Component that Pushes a `TagComponent` when the given key is
    /// hit — used to exercise dedup. Distinct concrete type from
    /// `TagComponent` so the compositor's TypeId-based dedup does
    /// not conflate them.
    struct Spawner {
        label: &'static str,
        spawn_key: KeyCode,
        spawn_label: &'static str,
    }

    impl Component for Spawner {
        fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
            if event.code == self.spawn_key {
                win.open(Box::new(TagComponent(self.spawn_label)));
            }
        }
        fn render(&self, _: &mut window::RenderWindow) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![(self.label, "")]
        }
    }

    /// Pushing a component whose concrete type matches something
    /// already on the stack is idempotent: focus moves to the
    /// existing one, no duplicate entry. Verifies the fix for
    /// "`i` with base focused while wiki is already open spawns
    /// wiki2". Dedup is by `Any::type_id`, so plugin authors don't
    /// have to declare identity — the type *is* the identity.
    #[test]
    fn push_with_existing_type_focuses_existing() {
        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        };

        let mut c = Compositor::new();
        c.push(Box::new(Spawner {
            label: "base",
            spawn_key: KeyCode::Char('i'),
            spawn_label: "wiki",
        }));
        // Open the spawned component for the first time.
        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2);
        assert_eq!(focused_tag(&c), "wiki");

        // Tab back to base.
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");

        // Press `i` again — would spawn a second TagComponent, but
        // the compositor sees TypeId collision and focuses the
        // existing instance. Stack length stays 2.
        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2, "no duplicate of same type in stack");
        assert_eq!(focused_tag(&c), "wiki", "focus moves to existing instance");
    }

    /// Component that calls `win.toggle(c)` (instead of `win.open(c)`)
    /// for a given key. Used to exercise the toggle-API close path.
    struct Toggler {
        label: &'static str,
        spawn_key: KeyCode,
        spawn_label: &'static str,
    }

    impl Component for Toggler {
        fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
            if event.code == self.spawn_key {
                win.toggle(Box::new(TagComponent(self.spawn_label)));
            }
        }
        fn render(&self, _: &mut window::RenderWindow) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![(self.label, "")]
        }
    }

    /// Test component that overrides [`Component::dedup_tag`] to a
    /// per-instance string. Mirrors how [`crate::lua::LuaComponent`]
    /// differentiates Lua plugins despite sharing a Rust type — same
    /// concrete struct, distinct identity per instance via the tag.
    struct TaggedComponent {
        label: &'static str,
        tag: &'static str,
    }

    impl Component for TaggedComponent {
        fn handle_event(&mut self, _: KeyEvent, _: &mut Window) {}
        fn render(&self, _: &mut window::RenderWindow) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![(self.label, "")]
        }
        fn dedup_tag(&self) -> Option<&str> {
            Some(self.tag)
        }
    }

    /// Toggler that spawns `TaggedComponent`s — `a` → tag-a, `b` →
    /// tag-b. Used to drive the full `push_or_toggle` path through
    /// `win.toggle` while exercising the new dedup-tag identity.
    struct MultiToggler;

    impl Component for MultiToggler {
        fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
            let component = match event.code {
                KeyCode::Char('a') => Some(("a", "tag-a")),
                KeyCode::Char('b') => Some(("b", "tag-b")),
                _ => None,
            };
            if let Some((label, tag)) = component {
                win.toggle(Box::new(TaggedComponent { label, tag }));
            }
        }
        fn render(&self, _: &mut window::RenderWindow) {}
        fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
            vec![("base", "")]
        }
    }

    /// Two components sharing a Rust type (`TaggedComponent`) but
    /// declaring different `dedup_tag` values must both end up on
    /// the stack — the regression that motivated #188 and the
    /// dedup-tag override. Without the fix, the second toggle would
    /// hit the TypeId match path, evict the first, and drop the
    /// second.
    #[test]
    fn dedup_tag_lets_distinct_tags_coexist_under_toggle() {
        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        };
        let mut c = Compositor::new();
        c.push(Box::new(MultiToggler));

        // Toggle 'a' — tag-a lands on top.
        c.handle_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2);
        assert_eq!(focused_tag(&c), "a");

        // Cycle back to base so the next handle_event reaches MultiToggler.
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");

        // Toggle 'b' — distinct tag, must coexist with tag-a.
        c.handle_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE), &ctx);
        assert_eq!(
            c.len(),
            3,
            "different dedup_tag values must coexist on the stack",
        );
        assert_eq!(focused_tag(&c), "b");
    }

    /// Re-toggling the same `dedup_tag` still closes — same-identity
    /// dedup is preserved. This is the path "Toggle ISS pressed
    /// while ISS is already open" exercises in production.
    #[test]
    fn dedup_tag_matching_tags_close_under_toggle() {
        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        };
        let mut c = Compositor::new();
        c.push(Box::new(MultiToggler));

        c.handle_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2);
        c.cycle(true);

        // Second 'a' — same tag, closes the existing one.
        c.handle_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 1, "same dedup_tag must close existing");
        assert_eq!(focused_tag(&c), "base");
    }

    /// `win.toggle(c)` closes an existing instance of the same
    /// concrete type instead of refocusing it — the semantic palette
    /// entries labelled "Toggle X" need. Mirror of the refocus test
    /// above, same setup / different op.
    #[test]
    fn toggle_with_existing_type_closes_existing() {
        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        };

        let mut c = Compositor::new();
        c.push(Box::new(Toggler {
            label: "base",
            spawn_key: KeyCode::Char('i'),
            spawn_label: "wiki",
        }));
        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 2);
        assert_eq!(focused_tag(&c), "wiki");

        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");

        c.handle_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &ctx);
        assert_eq!(c.len(), 1, "toggle op closes existing of same type");
        assert_eq!(focused_tag(&c), "base", "focus returns to base layer");
    }

    /// Tab delivery is framework-level: even a component that
    /// consumes every key (the "bad plugin" case) can't block it.
    /// This is the structural guarantee that replaced the per-plugin
    /// "remember to Ignore Tab" contract.
    #[test]
    fn tab_is_intercepted_before_components() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        /// Absorbs every event and emits a no-op msg — the "bad
        /// plugin" that would swallow Tab if Tab reached it.
        struct SwallowsAll;
        impl Component for SwallowsAll {
            fn handle_event(&mut self, _: KeyEvent, win: &mut Window) {
                win.emit(AppMsg::Map(crate::map::Action::None));
            }
            fn render(&self, _: &mut window::RenderWindow) {}
        }

        let ctx = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        };

        let mut c = Compositor::new();
        c.push(Box::new(SwallowsAll));

        let msgs = c.handle_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &ctx);
        assert_eq!(msgs, vec![AppMsg::CycleFocus(true)]);

        let msgs = c.handle_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), &ctx);
        assert_eq!(msgs, vec![AppMsg::CycleFocus(false)]);
    }
}
