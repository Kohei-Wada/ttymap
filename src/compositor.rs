//! Compositor — stack-based focus/modal system (helix-inspired).
//!
//! Replaces the `FocusManager` + `FocusSurface` + `Plugin` trio with a
//! single primitive: a stack of [`Component`]s. The top of the stack
//! holds focus; push on activation, pop on [`EventResult::Close`].
//! Object lifetime *is* the visibility lifecycle, so plugins never
//! have to maintain a separate `is_visible` / `activate` / `deactivate`
//! contract — fresh instances on every push, dropped on every pop.
//!
//! The map's always-on overlays (wiki markers, etc.) live through
//! [`Painter`], a sibling trait with no focus semantics: painters are
//! drawn every frame regardless of what's on the compositor stack.
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

#![allow(dead_code)] // wired in progressively over the refactor

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
pub enum EventResult {
    Ignored,
    Consumed(Vec<AppMsg>),
    Close(Vec<AppMsg>),
    Push(Box<dyn Component>, Vec<AppMsg>),
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

    pub fn pop(&mut self) -> Option<Box<dyn Component>> {
        self.stack.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
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

    /// Footer hints from the top of the stack, or empty when nothing
    /// is above the bottom layer (caller falls back to its own hints).
    pub fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.stack
            .last()
            .map(|c| c.footer_hints())
            .unwrap_or_default()
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Map painters (always-on, focus-independent) ────────────────────

/// World-space overlay drawn every frame regardless of focus. Kept
/// separate from [`Component`] so a plugin's markers (e.g. wiki)
/// persist on the map while its panel is closed. Plugins that need
/// both implement both traits and share state through
/// `Rc<RefCell<_>>`.
pub trait Painter {
    fn paint(&self, p: &mut MapPainter<'_>);
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
/// activates the corresponding surface.
pub type SpawnComponent = Box<dyn Fn() -> Box<dyn Component>>;

/// Closure that kicks off a headless action (typically: start an
/// async `Task` or emit one-shot `AppMsg`s) when a palette entry is
/// selected.
pub type RunAction = Box<dyn Fn() -> Vec<AppMsg>>;

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
/// channel is optional — headless plugins add a task + palette
/// entry; visual plugins add an activation + palette entry; wiki
/// adds all four.
#[derive(Default)]
pub struct Registrar {
    pub painters: Vec<Box<dyn Painter>>,
    pub activations: Vec<Activation>,
    pub palette_entries: Vec<PaletteEntry>,
    pub tasks: Vec<Box<dyn Task>>,
}

impl Registrar {
    pub fn add_painter(&mut self, p: Box<dyn Painter>) {
        self.painters.push(p);
    }
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
