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
pub mod window;

pub use base::BaseLayer;

use std::any::Any;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::geo::LonLat;
use crate::plugin_api::MapApi;
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
    /// [`MapApi::cursor_ll`](crate::plugin_api::MapApi::cursor_ll)
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
}

use window::{Window, WindowOps};

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
    /// `close` → `opens` (TypeId dedup → refocus) → `toggles`
    /// (TypeId dedup → close) → return `msgs` for the caller to
    /// dispatch. See [`window`] module docs.
    fn apply_ops(&mut self, idx: usize, ops: WindowOps) -> Vec<AppMsg> {
        if ops.close {
            self.stack.remove(idx);
            self.clamp_focus_after_shrink();
        }
        for c in ops.opens {
            self.push_or_focus(c);
        }
        for c in ops.toggles {
            self.push_or_toggle(c);
        }
        ops.msgs
    }

    /// Push `c` on top **unless** a component of the same concrete
    /// type is already on the stack — in that case focus jumps to the
    /// existing instance and `c` is dropped. This makes repeated
    /// activation keys (e.g. `i` pressed twice with focus on the
    /// base between presses) idempotent instead of stacking duplicate
    /// panels. For toggle semantics (close-if-open) see
    /// [`push_or_toggle`](Self::push_or_toggle).
    ///
    /// Uses [`Any::type_id`] from the supertrait so no per-plugin
    /// declaration is needed — the concrete component type *is* its
    /// dedup identity. A plugin author cannot forget to opt in.
    fn push_or_focus(&mut self, c: Box<dyn Component>) {
        let new_type = Any::type_id(&*c);
        if let Some(existing) = self
            .stack
            .iter()
            .position(|s| Any::type_id(&**s) == new_type)
        {
            self.focused_idx = existing;
            return;
        }
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    /// Toggle counterpart to [`push_or_focus`]: if a component of the
    /// same concrete type is already on the stack, that existing
    /// instance closes and `c` is dropped. Otherwise `c` is pushed.
    ///
    /// Used by palette entries labelled "Toggle X" — pressing a
    /// second time should close the surface, not refocus it.
    /// Activation keys (`i`, `?`, …) still use `push_or_focus` so
    /// their refocus semantic is preserved.
    fn push_or_toggle(&mut self, c: Box<dyn Component>) {
        let new_type = Any::type_id(&*c);
        if let Some(existing) = self
            .stack
            .iter()
            .position(|s| Any::type_id(&**s) == new_type)
        {
            self.stack.remove(existing);
            self.clamp_focus_after_shrink();
            return;
        }
        self.stack.push(c);
        self.focused_idx = self.stack.len() - 1;
    }

    /// Poll every component; drain all queued `win.emit(...)` /
    /// `win.close()` / `win.open(...)` ops and apply them in the
    /// same way [`handle_event`](Self::handle_event) does. Always-on
    /// overlays poll too — they may have async work (geocoding,
    /// throttle ticks) but the only ops they emit are
    /// `win.emit(AppMsg)`; close/open are ignored on overlays
    /// because they aren't on the focusable stack.
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
            let mut ops = WindowOps::default();
            {
                let mut win = Window::new(&mut ops, ctx);
                c.poll(&mut win);
            }
            // Overlays only meaningfully emit; close/open are silently
            // dropped because overlays don't live on the focusable stack.
            all_msgs.extend(ops.msgs);
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
    /// Push a new component. TypeId dedup refocuses an existing
    /// instance of the same concrete type (search, palette sub-mode, …).
    Spawn(SpawnComponent),
    /// Toggle semantics: same as [`Spawn`](Self::Spawn) on first
    /// selection, but closes the existing instance on re-selection.
    /// Used by palette labels of the form "Toggle X".
    Toggle(SpawnComponent),
    /// Fire-and-forget action — selection dispatches `Vec<AppMsg>`
    /// without pushing a component. No in-tree caller after the
    /// plugin migration; kept as an extension point for future
    /// bridges that emit AppMsg directly from a palette entry.
    #[allow(dead_code)]
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
    /// Register a background task polled every tick. No in-tree
    /// caller after the plugin migration; kept as an extension
    /// point for future Lua bridge work or Rust-only plugins.
    #[allow(dead_code)]
    pub fn add_task(&mut self, t: Box<dyn Task>) {
        self.tasks.push(t);
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
            spawn: Box::new(move |ctx| Box::new(factory(ctx)) as Box<dyn Component>),
        });
    }

    /// Add a palette entry that toggles a component on/off — opens it
    /// on first selection, closes the existing instance on
    /// re-selection.
    pub fn add_toggle<F, C>(
        &mut self,
        label: impl Into<String>,
        hint: impl Into<String>,
        factory: F,
    ) where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            kind: PaletteKind::Toggle(Box::new(move |ctx| {
                Box::new(factory(ctx)) as Box<dyn Component>
            })),
        });
    }

    /// Add a palette entry that spawns a fresh instance every time —
    /// no toggle dedup. Use when the component is meant to be rebuilt
    /// per open (search, palette sub-modes).
    pub fn add_spawn<F, C>(&mut self, label: impl Into<String>, hint: impl Into<String>, factory: F)
    where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            kind: PaletteKind::Spawn(Box::new(move |ctx| {
                Box::new(factory(ctx)) as Box<dyn Component>
            })),
        });
    }

    /// Add a fire-and-forget palette entry — selecting it returns
    /// `Vec<AppMsg>` to dispatch, no component pushed. No in-tree
    /// caller after the plugin migration; kept as an extension
    /// point for future bridges.
    #[allow(dead_code)]
    pub fn add_run<F>(&mut self, label: impl Into<String>, hint: impl Into<String>, action: F)
    where
        F: Fn(&Context) -> Vec<AppMsg> + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            kind: PaletteKind::Run(Box::new(action)),
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
        self.overlays.push(Box::new(move |ctx| {
            Box::new(factory(ctx)) as Box<dyn Component>
        }));
    }
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
