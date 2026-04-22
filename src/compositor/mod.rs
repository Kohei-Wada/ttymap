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

/// Stack of modal components. Replaces `FocusManager`.
pub struct Compositor {
    stack: Vec<Box<dyn Component>>,
}

impl Compositor {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, c: Box<dyn Component>) {
        self.stack.push(c);
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    /// Deliver a key event top-to-bottom until something takes it.
    /// Returns the messages the handling component(s) emitted.
    /// Handles `Close` (pop) and `Push` (push new) by mutating the
    /// stack before returning.
    pub fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> Vec<AppMsg> {
        for i in (0..self.stack.len()).rev() {
            match self.stack[i].handle_event(event, ctx) {
                EventResult::Ignored => continue,
                EventResult::Consumed(msgs) => return msgs,
                EventResult::Close(msgs) => {
                    self.stack.remove(i);
                    return msgs;
                }
                EventResult::Push(new_component, msgs) => {
                    self.stack.push(new_component);
                    return msgs;
                }
                EventResult::CloseAndPush(new_component, msgs) => {
                    self.stack.remove(i);
                    self.stack.push(new_component);
                    return msgs;
                }
            }
        }
        Vec::new()
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

    /// Footer hints from the top of the stack, or empty when nothing
    /// is above the bottom layer (caller falls back to its own hints).
    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.stack
            .last()
            .map(|c| c.footer_hints())
            .unwrap_or_default()
    }

    /// Rotate the stack so Tab-style focus cycling works. Forward
    /// moves the top to the bottom (bringing the next component up);
    /// backward does the reverse. No-op with fewer than two
    /// components. The bottom-layer keymap is index 0, so it stays
    /// put unless there's only it (trivial case). This replaces
    /// `FocusManager::cycle`.
    ///
    /// Note: the bottom layer sits at index 0 and participates in
    /// rotation; reaching it via Tab equals the old "cycle to
    /// Background" state. Callers that want to preserve a
    /// never-rotate bottom can lift the first element out before
    /// rotating.
    pub fn cycle(&mut self, forward: bool) {
        if self.stack.len() <= 1 {
            return;
        }
        // Keep index 0 (the bottom layer) fixed; rotate only the
        // modals above it.
        if forward {
            if let Some(top) = self.stack.pop() {
                self.stack.insert(1, top);
            }
        } else {
            // Swap the topmost two (simplest back-cycle for up to a
            // handful of components; matches the old cycle semantics
            // of "previous visible plugin").
            let len = self.stack.len();
            self.stack.swap(len - 1, len - 2);
        }
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
