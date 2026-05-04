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
//! World-space map overlays (wiki markers etc.) are *not* a
//! `Component` concern. Every Lua plugin's per-frame map paint runs
//! through [`crate::lua::LuaEventBus::dispatch_tick`] (called from
//! [`crate::app::ui::draw`]) which hands the plugin a [`MapApi`] it
//! draws into directly — tying map-side rendering to plugin
//! lifetime is plugin-side policy (a captured `CardHandle` that's
//! nil while closed), not a framework hook.
//!
//! Plugin activation primitives ([`Activation`], [`PaletteEntry`])
//! live here; the Lua subsystem's [`crate::lua::Registrar`]
//! collection bucket bundles them with its own
//! [`crate::lua::LuaEventBus`] / [`crate::lua::api::LuaHostHandles`]
//! at plugin-load time. `App` takes the finished bundle and never
//! names a concrete plugin type. Compositor itself is unaware of
//! Lua — it speaks only `Activation` / `PaletteEntry` / `Component`.

pub mod base;
pub mod map_api;
pub mod op;
pub mod render;
mod sidebar;
pub mod window;

pub use base::BaseLayer;
pub use map_api::MapApi;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::UserCommand;
use crate::compositor::op::Op;
use crate::theme::ThemeId;

// ── Framework-reserved keys ────────────────────────────────────────

/// Keys the compositor handles globally, without consulting any
/// component:
/// - `Tab` / `C-j` → forward cycle
/// - `Shift-Tab` / `BackTab` / `C-k` → backward cycle
///
/// Intercepting here — rather than at `BaseLayer` — means no
/// component on the stack can accidentally absorb the key. Focus
/// cycling is a property of the framework, not of any plugin's
/// correctness.
///
/// `C-j` requires the kitty keyboard protocol's
/// `DISAMBIGUATE_ESCAPE_CODES` flag (pushed at startup in
/// `main`); otherwise terminals collapse `C-j` onto `Enter` (=
/// ASCII LF) and the binding silently does nothing. `C-k` has no
/// such legacy collision and works regardless. Tab / Shift-Tab
/// always work as a fallback.
fn intercept_focus_key(event: KeyEvent) -> Option<UserCommand> {
    if event.code == KeyCode::Tab && event.modifiers == KeyModifiers::NONE {
        return Some(UserCommand::CycleFocus(true));
    }
    if event.code == KeyCode::BackTab
        || (event.code == KeyCode::Tab && event.modifiers.contains(KeyModifiers::SHIFT))
    {
        return Some(UserCommand::CycleFocus(false));
    }
    let only_ctrl = event.modifiers == KeyModifiers::CONTROL;
    if only_ctrl && event.code == KeyCode::Char('j') {
        return Some(UserCommand::CycleFocus(true));
    }
    if only_ctrl && event.code == KeyCode::Char('k') {
        return Some(UserCommand::CycleFocus(false));
    }
    None
}

// ── Stable component identity ──────────────────────────────────────

/// Stable identity for a component on the [`Compositor`] stack.
///
/// Allocated by [`CardId::next`] from a process-global atomic counter
/// so external actors (Lua handles, future async sources) can reserve
/// an id at the call site that opens a card and use it later to
/// request a close — even though the actual push may have applied on
/// a later iteration. Uniqueness across the program lifetime; we
/// never recycle ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CardId(u64);

impl CardId {
    /// Allocate a fresh `CardId`. Single atomic increment; the
    /// counter is process-global so any caller (compositor's own
    /// `push`, Lua bridge's `api.card.open`) gets a unique id without
    /// coordination.
    pub fn next() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

// ── Component + event routing ──────────────────────────────────────

/// Read-only snapshot of app-level context a component may need
/// during a hook. Reached by the component through
/// [`Window::ctx`](window::Window::ctx).
#[derive(Debug, Clone, Copy)]
pub struct Context {
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
/// nvim-style: the compositor never deduplicates pushes. Pressing an
/// activation key twice produces two instances of the plugin on the
/// stack. Plugins that want toggle behavior implement self-close in
/// their own `handle_event` (return `win.close()` when the activation
/// key fires while focused).
///
/// The event-producing hooks ([`handle_event`](Self::handle_event)
/// and [`poll`](Self::poll)) receive a
/// [`&mut Window`](window::Window) and express intent through it
/// (`win.close()`, `win.open(c)`, `win.emit(msg)`, `win.ignore()`).
/// The framework applies those ops atomically after the hook
/// returns, so components cannot break stack / focus invariants
/// regardless of what order they call the methods.
///
/// Where the component renders. Today only two slots:
///
/// - `Floating`: drawn over the map area, on top of everything.
///   Used for the command palette (the only floating component
///   in tree). Lua plugins are *not* allowed to be Floating —
///   the spec the bridge accepts always lands in `Sidebar`.
/// - `Sidebar`: shares the left sidebar via equal vertical split
///   so multiple sections can live there at once.
///
/// Default is `Floating` because the only Rust-side `Component`
/// impl that doesn't override is the palette, and palette is the
/// canonical floating component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Floating,
    Sidebar,
}

pub trait Component {
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
    /// Default impl is no-op — for components that exist only to
    /// hold focus / poll async work, with no sidebar UI of their
    /// own.
    fn render(&self, _win: &mut window::RenderWindow) {}

    /// Where to draw this component. Defaults to
    /// [`Placement::Floating`] (free-floating panel over the
    /// map). Plugins that want to live in the left sidebar
    /// override to [`Placement::Sidebar`].
    fn placement(&self) -> Placement {
        Placement::Floating
    }

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
    pub(super) stack: Vec<(CardId, Box<dyn Component>)>,
    /// Index of the component that receives key events first.
    /// Invariant: `focused_idx < stack.len()` whenever the stack is
    /// non-empty. After every push, this becomes the new top; after
    /// a close/pop it is clamped down to the new last index.
    pub(super) focused_idx: usize,
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            focused_idx: 0,
        }
    }

    /// Push a component, allocating a fresh [`CardId`] internally.
    /// Used for in-process pushes whose caller doesn't need to
    /// reference the id later (e.g. [`BaseLayer`] at startup,
    /// palette at startup, or `Window::open` from inside a key
    /// handler).
    pub fn push(&mut self, c: Box<dyn Component>) -> CardId {
        let id = CardId::next();
        self.push_with_id(id, c);
        id
    }

    /// Push a component with a caller-supplied id. Used by external
    /// sources (Lua `api.card.open`) that need to reserve the id at
    /// the call site so they can return a handle whose close path
    /// targets this specific component.
    pub fn push_with_id(&mut self, id: CardId, c: Box<dyn Component>) {
        self.stack.push((id, c));
        self.focused_idx = self.stack.len() - 1;
    }

    /// Pop the component matching `id` off the stack. Silent no-op
    /// when `id` isn't present (handle closed twice, or component
    /// already self-closed via `win.close()`).
    pub fn close_by_id(&mut self, id: CardId) {
        if let Some(idx) = self.stack.iter().position(|(i, _)| *i == id) {
            self.stack.remove(idx);
            self.clamp_focus_after_shrink();
        }
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
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
    /// `Tab` / `Shift-Tab` / `BackTab` / `C-j` / `C-k` are
    /// **framework-reserved** for focus cycling. `q` is *not*
    /// framework-reserved: each plugin binds it themselves so the
    /// plugin's own close handler runs (resetting per-plugin state
    /// like a `w` window handle, an `enabled` feed flag, …).
    /// Closing from the outside via `stack.remove` would leave
    /// the lua-side state pointing at a stale handle.
    pub fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> Vec<Op> {
        if let Some(msg) = intercept_focus_key(event) {
            return vec![Op::Command(msg)];
        }
        if self.stack.is_empty() {
            return Vec::new();
        }
        let focused = self.focused_idx;
        let focused_id = self.stack[focused].0;
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, ctx, focused_id);
            self.stack[focused].1.handle_event(event, &mut win);
        }
        // Fall-through: only when the hook queued nothing *and*
        // explicitly called `ignore()`, and the focus isn't already
        // on the base layer.
        if ops.is_ignorable_noop() && ops.ignored && focused != 0 {
            let base_id = self.stack[0].0;
            let mut ops = WindowOps::default();
            {
                let mut win = Window::new(&mut ops, ctx, base_id);
                self.stack[0].1.handle_event(event, &mut win);
            }
            return ops.ops;
        }
        ops.ops
    }

    /// Poll every component. Intent emissions queued by the hook
    /// (`win.emit`) ride the same [`WindowOps`] as stack ops; the
    /// concatenated [`Op`] vec is returned for App to apply.
    pub fn poll(&mut self, ctx: &Context) -> Vec<Op> {
        // Walk in reverse so closing a component doesn't disturb
        // indices of later ones. Collect ops per index first; the
        // caller applies them in arrival order.
        let mut all = Vec::new();
        let len = self.stack.len();
        for i in (0..len).rev() {
            let id = self.stack[i].0;
            let mut ops = WindowOps::default();
            {
                let mut win = Window::new(&mut ops, ctx, id);
                self.stack[i].1.poll(&mut win);
            }
            all.extend(ops.ops);
        }
        all
    }

    /// Whether focus is on the map (i.e. the [`BaseLayer`] at
    /// stack index 0). Drives the world frame's border highlight
    /// in the UI: when nothing is pushed, focus stays on base; as
    /// soon as a modal / sidebar component is pushed, focus moves
    /// to it and the world border dims.
    pub fn is_base_focused(&self) -> bool {
        self.focused_idx == 0
    }

    /// Count of `Placement::Sidebar` components on the stack.
    ///
    /// Used by the App's auto-open logic — the sidebar opens
    /// on a *count increase*, not on the existence of any sidebar
    /// component, so toggling the sidebar off via `\` doesn't
    /// fight per-frame auto-open while components stay alive.
    /// Also used by the UI layer (`> 0`) to decide whether to show
    /// the "(no sections yet)" placeholder when the sidebar is open
    /// but empty.
    pub fn sidebar_component_count(&self) -> usize {
        self.stack
            .iter()
            .filter(|(_, c)| c.placement() == Placement::Sidebar)
            .count()
    }

    /// Footer hints from the currently focused component.
    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.stack
            .get(self.focused_idx)
            .map(|(_, c)| c.footer_hints())
            .unwrap_or_default()
    }

    /// Name of the currently focused component (empty when the focus
    /// is on the base layer or any component that opted not to label
    /// itself). Surfaced in the footer so the user can tell which
    /// plugin is consuming their keystrokes when modals stack.
    pub fn focused_name(&self) -> &'static str {
        self.stack
            .get(self.focused_idx)
            .map(|(_, c)| c.name())
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

// ── Plugin activation primitives ───────────────────────────────────

/// Factory closure producing a fresh [`Component`] when the user
/// activates the corresponding surface. Receives a [`Context`]
/// snapshot so plugins that read app-level state at activation time
/// (e.g. palette seeds its "(current)" theme hint from `theme_id`)
/// can do so without a separate lifecycle hook.
///
/// Returns `None` when the factory wants to skip the push entirely
/// — used by Lua plugins whose activation callback returned a falsy
/// value, signalling "I read my state and decided not to open this
/// time".
pub type SpawnComponent = Box<dyn Fn(&Context) -> Option<Box<dyn Component>>>;

/// One activation entry — "when this key is pressed while nothing
/// modal is above the bottom layer, invoke `spawn` and push the
/// result". Collected by [`crate::lua::Registrar`] at plugin-load
/// time and consumed by [`BaseLayer`] at startup.
pub struct Activation {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub spawn: SpawnComponent,
}

/// Palette entry description. Selection always pushes a fresh
/// component on the stack — there's no toggle/spawn distinction now
/// that the compositor doesn't dedup. A plugin that wants "close on
/// re-select" closes itself in its own `handle_event`.
pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    /// Plugin's canonical short name (`module.name`). Used as the
    /// footer slug paired with `hint` (`[<hint> <name>]`).
    pub name: &'static str,
    pub spawn: SpawnComponent,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test component that identifies itself via
    /// `footer_hints`. Distinct string parameters are just labels.
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

    /// Component that pushes a `TagComponent` when the given key is
    /// hit — used to exercise the no-dedup invariant: a fresh
    /// instance is stacked on every activation.
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

    /// Build a disposable `(Sender, Receiver)` pair for tests that
    /// Drive a key event into the compositor and apply the returned
    /// ops the same way App would: stack mutations are applied
    /// to `c` itself, intents are returned to the test for assertion.
    fn drive(c: &mut Compositor, event: KeyEvent, ctx: &Context) -> Vec<UserCommand> {
        let ops = c.handle_event(event, ctx);
        let mut intents = Vec::new();
        for op in ops {
            match op {
                Op::Push { id, component } => c.push_with_id(id, component),
                Op::Close(id) => c.close_by_id(id),
                Op::Command(intent) => intents.push(intent),
            }
        }
        intents
    }

    /// `open` always pushes a new instance, even if a component
    /// with the same concrete type is already on the stack. nvim-
    /// style: no Rust-side identity dedup. A plugin that wants
    /// "open or focus existing" semantics implements that itself.
    #[test]
    fn push_always_stacks_new_instance() {
        let ctx = Context {
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
        drive(
            &mut c,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &ctx,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(focused_tag(&c), "wiki");

        // Tab back to base.
        c.cycle(true);
        assert_eq!(focused_tag(&c), "base");

        // Press `i` again — pushes a second wiki on top. Plugins
        // that want toggle behavior implement self-close in their
        // own handle_event.
        drive(
            &mut c,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &ctx,
        );
        assert_eq!(c.len(), 3, "second activation pushes a new instance");
        assert_eq!(focused_tag(&c), "wiki");
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
                win.emit(UserCommand::Map(crate::core::map::MapAction::None));
            }
            fn render(&self, _: &mut window::RenderWindow) {}
        }

        let ctx = Context {
            theme_id: ThemeId::Dark,
            cursor: None,
        };

        let mut c = Compositor::new();
        c.push(Box::new(SwallowsAll));

        let intents = drive(
            &mut c,
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            &ctx,
        );
        assert_eq!(intents, vec![UserCommand::CycleFocus(true)]);

        let intents = drive(
            &mut c,
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            &ctx,
        );
        assert_eq!(intents, vec![UserCommand::CycleFocus(false)]);
    }
}
