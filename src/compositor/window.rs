//! Window API — capability-constrained handle passed to components.
//!
//! **Prototype.** Compiles (so types are validated) but not wired
//! into `Compositor` yet. Documents the target shape for replacing
//! the `EventResult`-return-value Component API with an imperative
//! `&mut Window` call API.
//!
//! # Why
//!
//! Today a component's `handle_event` returns a `Vec<AppMsg>` + an
//! `EventResult` variant describing what the compositor should do
//! with the stack (`Consumed` / `Close` / `Push` / `CloseAndPush` /
//! `Ignored`). Compound operations like "close me and open a sibling"
//! need their own variant (`CloseAndPush`). Adding a new compound
//! requires a new variant.
//!
//! With a `Window` handle, the component expresses the same intents
//! imperatively:
//!
//! ```ignore
//! fn handle_event(&mut self, ev: KeyEvent, win: &mut Window) {
//!     if esc {
//!         win.close();
//!     } else if picked_search {
//!         win.close();
//!         win.open(Box::new(SearchComponent::new()));
//!     } else if picked_theme(theme) {
//!         win.emit(AppMsg::SetTheme(theme));
//!         win.close();
//!     }
//! }
//! ```
//!
//! The `close()` / `open()` / `emit()` calls are **queued into
//! `WindowOps`**. When `handle_event` returns, the compositor reads
//! the queue and applies it atomically in a deterministic order
//! (close → push → msgs). The component never holds `&mut
//! Compositor`, so invariants remain enforced by the framework.
//!
//! # What this gains over EventResult
//!
//! - **No variant explosion.** `CloseAndPush` collapses into
//!   `win.close(); win.open(c);` — two atomic queued ops, compositor
//!   applies them in order. Future compounds are free.
//! - **Theme injection goes away.** `win.theme()` exposes theme
//!   without a `theme: &UiTheme` argument threaded through every
//!   render path.
//! - **Uniform draw primitives.** `win.popup(...)`, `win.list(...)`,
//!   `win.text(...)` factor the `Clear + Block + centered_rect +
//!   Paragraph` boilerplate currently duplicated across every
//!   plugin's panel.rs.
//!
//! # What it doesn't lose
//!
//! Plugin still cannot mutate `Compositor` directly — `Window` is a
//! narrow capability type. Every method queues into `WindowOps`;
//! compositor applies with dedup, clamping, and ordering rules. The
//! "plugin can't break focus/stack invariants" guarantee is
//! preserved.

#![allow(dead_code)] // prototype; wired in incrementally

use std::any::Any;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::compositor::Context;
use crate::theme::UiTheme;

// ── WindowOps: the queue ───────────────────────────────────────────

/// Queue of actions the compositor applies after `Component::*`
/// returns. Plugin never sees this type directly; it mutates it only
/// through [`Window`] methods.
#[derive(Default)]
pub struct WindowOps {
    /// `true` if the plugin called `win.close()`. Close fires before
    /// any queued `open`s, so a plugin that does `close(); open(x);`
    /// in one `handle_event` ends with `x` in its slot.
    pub close: bool,
    /// Components queued by `win.open(c)`. Pushed in the order they
    /// were queued. Dedup (by `TypeId`) is applied when the
    /// compositor drains the queue.
    pub opens: Vec<Box<dyn Component>>,
    /// Messages for `App::dispatch`. Emitted in the order queued.
    /// Typically dispatched **before** the stack is mutated so e.g.
    /// a `Jump` fires while the current focus is still valid.
    pub msgs: Vec<AppMsg>,
    /// `true` if the plugin explicitly returned "I don't handle
    /// this". Triggers fall-through to the base layer (if this
    /// component isn't already the base).
    pub ignored: bool,
}

impl WindowOps {
    /// Compositor application order (documented here, not
    /// implemented by this prototype):
    ///
    /// 1. `msgs` → forwarded to `App::dispatch` before stack
    ///    mutations so user-visible effects (Jump, SetTheme) fire
    ///    while the current component is still conceptually alive.
    /// 2. `close` → if true, pop the calling component.
    /// 3. `opens` → each is pushed in order; each push goes through
    ///    TypeId dedup (existing instance → focus moves, new drop).
    /// 4. `ignored` → only honoured if `opens` empty and `close`
    ///    false and `msgs` empty. Otherwise the plugin *did*
    ///    express intent, so `ignored` is a contradiction and is
    ///    silently dropped.
    pub fn is_noop(&self) -> bool {
        !self.close && self.opens.is_empty() && self.msgs.is_empty() && !self.ignored
    }
}

// ── Window: the capability type ────────────────────────────────────

/// Handle plugins receive in place of returning `EventResult`.
/// Constrained by construction: no `&mut Compositor`, no direct
/// access to the stack, no way to reorder siblings.
pub struct Window<'a> {
    ops: &'a mut WindowOps,
    ctx: &'a Context,
    theme: &'a UiTheme,
    frame: Option<&'a mut Frame<'a>>, // only populated during render
    area: Rect,
}

impl<'a> Window<'a> {
    /// Constructor used by the compositor when delivering events —
    /// prototype-only; real version will be crate-private.
    pub fn new_for_event(ops: &'a mut WindowOps, ctx: &'a Context, theme: &'a UiTheme) -> Self {
        Self {
            ops,
            ctx,
            theme,
            frame: None,
            area: Rect::default(),
        }
    }

    /// Constructor used by the compositor during render — prototype-
    /// only. Real version will be crate-private and set frame/area.
    pub fn new_for_render(
        ops: &'a mut WindowOps,
        ctx: &'a Context,
        theme: &'a UiTheme,
        frame: &'a mut Frame<'a>,
        area: Rect,
    ) -> Self {
        Self {
            ops,
            ctx,
            theme,
            frame: Some(frame),
            area,
        }
    }

    // ── Read-only accessors (no capability) ───────────────────────

    pub fn theme(&self) -> &UiTheme {
        self.theme
    }

    pub fn ctx(&self) -> &Context {
        self.ctx
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    // ── Queued actions (compositor applies later) ─────────────────

    /// Pop this component from the compositor stack after
    /// `handle_event` returns. Idempotent — second call does
    /// nothing. Atomic with `open` calls in the same event (close
    /// fires first).
    pub fn close(&mut self) {
        self.ops.close = true;
    }

    /// Push a new component on top of the compositor stack after
    /// `handle_event` returns. If `win.close()` was also called this
    /// event, the stack ends up with `c` replacing the current
    /// component — the equivalent of the old
    /// `EventResult::CloseAndPush`. If not, `c` sits on top of the
    /// current component — the equivalent of `EventResult::Push`.
    pub fn open(&mut self, c: Box<dyn Component>) {
        self.ops.opens.push(c);
    }

    /// Queue a message for `App::dispatch`. Emitted before stack
    /// mutations settle, so downstream state changes happen while
    /// the calling component is still on the stack (e.g. a `Jump`
    /// fires first, then `close()` pops the picker).
    pub fn emit(&mut self, msg: AppMsg) {
        self.ops.msgs.push(msg);
    }

    /// Mark this event as not-mine; compositor re-delivers to the
    /// base layer (if this isn't already it). Only meaningful when
    /// no other op was queued — mixing with `close` / `open` /
    /// `emit` is silently dropped.
    pub fn ignore(&mut self) {
        self.ops.ignored = true;
    }

    // ── Draw primitives (only meaningful during render) ───────────
    //
    // These are stubs in the prototype. Real versions will wrap
    // ratatui constructs (`Clear` + `Block` + `Paragraph` + layout
    // math) with theme applied automatically.

    /// Fill a sub-area with a theme-styled bordered popup containing
    /// the given content. Convenience for `Clear + Block +
    /// Paragraph`. Takes a relative rect inside `self.area()`.
    pub fn popup(&mut self, _rect: Rect, _title: &str, _content: &str) {
        // TODO prototype — real impl uses self.frame.as_mut().unwrap()
    }

    /// Render a theme-styled selectable list. Higher-level than a
    /// raw `List` widget because it handles highlight style and
    /// wrapping.
    pub fn list<'i, I: IntoIterator<Item = &'i str>>(
        &mut self,
        _rect: Rect,
        _items: I,
        _selected: usize,
    ) {
        // TODO prototype
    }

    /// Render a single line of theme-styled text.
    pub fn text(&mut self, _rect: Rect, _text: &str) {
        // TODO prototype
    }
}

// ── Component trait (new shape) ────────────────────────────────────

/// Prototype of the post-Window-API `Component` trait. **Not yet
/// replacing [`crate::compositor::Component`]** — coexists during
/// migration; plugins will be converted one at a time.
///
/// Every hook receives a `&mut Window` instead of returning
/// `EventResult`. The plugin expresses intent through `win.*` calls.
pub trait Component: Any {
    /// Handle a single key event. Plugin queues actions via `win`.
    /// If no `win.*` call was made, the event is implicitly ignored.
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window);

    /// Paint this component's popup / panel into `win.area()`.
    /// Plugin calls `win.popup(...)` etc. to render; theme is
    /// automatically applied.
    fn render(&self, win: &mut Window);

    /// Paint world-space primitives on the map. Called every frame
    /// while the component is on the stack; gated on stack presence
    /// exactly like `render`.
    fn paint_on_map(&self, _win: &mut Window) {}

    /// Periodic tick for async work. Plugin drains completed
    /// futures and emits messages via `win.emit(...)`.
    fn poll(&mut self, _win: &mut Window) {}

    /// Footer hints shown while this component is focused.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

// ── Prototype: what SearchComponent looks like after conversion ────
//
// Mirrors the real `plugin::search::SearchComponent` — just the
// handle_event path to show how Close/Push collapse.

use crate::geo::LonLat as _LonLat;

struct StubSearchResult {
    location: _LonLat,
}

pub struct SearchComponent {
    query: String,
    candidates: Vec<StubSearchResult>,
    selected: usize,
}

impl SearchComponent {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
        }
    }

    fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }
}

impl Default for SearchComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for SearchComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        use crossterm::event::{KeyCode, KeyModifiers};

        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(event.code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && event.code == KeyCode::Char('p'));
            let down = matches!(event.code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && event.code == KeyCode::Char('n'));

            if event.code == KeyCode::Esc {
                win.close();
            } else if event.code == KeyCode::Enter {
                let loc = self.candidates[self.selected].location;
                win.emit(AppMsg::Jump(loc));
                win.close();
            } else if up && self.selected > 0 {
                self.selected -= 1;
            } else if down && self.selected + 1 < self.candidates.len() {
                self.selected += 1;
            }
            return;
        }

        match event.code {
            KeyCode::Esc => win.close(),
            KeyCode::Enter if self.query.is_empty() => win.close(),
            KeyCode::Enter => { /* kick off async search; no stack op */ }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
            }
            KeyCode::Char(c) => self.query.push(c),
            _ => { /* modal */ }
        }
    }

    fn render(&self, win: &mut Window) {
        // Real: win.popup(...), win.list(...) etc.
        let _ = win;
    }
}

// ── Prototype: what PaletteComponent's "open search" looks like ────
//
// Shows how CloseAndPush collapses. The entire PaletteAction::Push
// variant goes away.
//
// ```ignore
// // Before (EventResult world):
// PaletteAction::Push(component) => EventResult::CloseAndPush(component, Vec::new()),
//
// // After (Window world):
// fn handle_event(&mut self, ev, win) {
//     match ev.code {
//         KeyCode::Enter => match self.provider.execute(idx, win.ctx()) {
//             PaletteAction::Close => win.close(),
//             PaletteAction::Run(msgs) => {
//                 for m in msgs { win.emit(m); }
//                 win.close();
//             }
//             PaletteAction::Push(c) => {
//                 // No new EventResult variant — just close + open.
//                 win.close();
//                 win.open(c);
//             }
//             PaletteAction::SwitchProvider(p) => { self.provider = p; }
//         },
//         ...
//     }
// }
// ```
