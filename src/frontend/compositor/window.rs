//! [`Window`] — capability-constrained handle passed to components.
//!
//! Components receive a `&mut Window` on every hook (`handle_event`
//! and `poll`). They express intent by calling methods on it:
//!
//! ```ignore
//! fn handle_event(&mut self, ev: KeyEvent, win: &mut Window) {
//!     if ev.code == KeyCode::Esc {
//!         win.close();
//!     } else if enter_with_selection {
//!         win.emit(UserIntent::Map(Action::Jump(loc)));
//!         win.close();
//!     } else if ev.code == KeyCode::Char('/') {
//!         win.close();
//!         win.open(Box::new(SearchComponent::new()));
//!     }
//! }
//! ```
//!
//! Stack-mutation methods (`close`, `open`, `ignore`) queue into
//! [`WindowOps`]; the compositor drains the queue after the hook
//! returns and applies them in a deterministic order (`close` →
//! `opens`). `emit` is *not* queued — it routes the [`UserIntent`]
//! straight onto the App-level [`AppEvent`] channel, so every
//! intent (whether produced by the keymap, a Lua palette callback,
//! or a panel hook) flows through the same bus the render thread,
//! input thread, and Lua plugins push into. There's no longer a
//! second "synchronous return" path back to `App::dispatch`.
//!
//! # Why a handle instead of a return value
//!
//! Returning an `EventResult` enum with one variant per op
//! combination (`Close`, `Push`, `CloseAndPush`, …) does not scale
//! — every new compound op needs a new variant. The handle queues
//! primitive ops so compounds are expressed by composition. Plugin
//! still cannot hold `&mut Compositor` or mutate the stack directly;
//! the compositor is the sole applier of the queue, so invariants
//! (focus, dedup, clamp) remain framework-enforced.

use std::sync::mpsc;

use ratatui::Frame;
use ratatui::layout::{Margin, Rect};
use ratatui::style::Style;
use ratatui::widgets::{
    Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
};

use crate::frontend::compositor::{Component, Context};
use crate::frontend::{AppEvent, UserIntent};
use crate::theme::{StyleKind, UiTheme};

/// Queue of stack-mutation actions a [`Component`] hook recorded
/// through [`Window`]. Drained and applied by the compositor after
/// the hook returns. Intent emission (`win.emit`) is **not** queued
/// here — it fires directly onto the App's [`AppEvent`] bus.
#[derive(Default)]
pub(crate) struct WindowOps {
    /// `true` if the plugin called [`Window::close`]. Pops the
    /// calling component. Applied before `opens` so `close + open`
    /// replaces the component in the stack slot.
    pub close: bool,
    /// Components queued by [`Window::open`]. Each pushes a fresh
    /// stack entry — nvim-style, no identity dedup. Plugins that
    /// want toggle semantics ("close if already open") handle that
    /// themselves in their own `handle_event`.
    pub opens: Vec<Box<dyn Component>>,
    /// `true` if the plugin called [`Window::emit`] at least once.
    /// Tracked so the fall-through logic in `Compositor::handle_event`
    /// can tell "the hook only signalled ignore()" from "the hook
    /// did emit something" — without this flag, an emit-then-ignore
    /// component would leak its key down to the base layer.
    pub did_emit: bool,
    /// `true` if the plugin called [`Window::ignore`]. Meaningful
    /// only when no other op was queued — in that case the
    /// compositor re-delivers the event to the base layer (unless
    /// the handler already was the base). Ignored otherwise.
    pub ignored: bool,
}

impl WindowOps {
    /// `true` iff the hook made no state-changing call. When the
    /// hook's only effect was `ignore()`, this is also true (ignore
    /// itself is a signal, not an op).
    pub(crate) fn is_ignorable_noop(&self) -> bool {
        !self.close && self.opens.is_empty() && !self.did_emit
    }
}

/// Handle components receive on every hook. Constrained by design:
/// no `&mut Compositor` is reachable through it, so components
/// cannot break focus / stack invariants even if buggy.
///
/// Read-only accessors (`ctx`) give the component what it needs for
/// decision-making without granting any capability. `emit` routes
/// onto the App-level [`AppEvent`] bus the compositor was given;
/// the component never sees the bus directly, so signature
/// stability survives changes to the channel topology.
pub struct Window<'a> {
    ops: &'a mut WindowOps,
    ctx: &'a Context,
    event_tx: &'a mpsc::Sender<AppEvent>,
}

impl<'a> Window<'a> {
    pub(crate) fn new(
        ops: &'a mut WindowOps,
        ctx: &'a Context,
        event_tx: &'a mpsc::Sender<AppEvent>,
    ) -> Self {
        Self { ops, ctx, event_tx }
    }

    /// App-level snapshot passed for this hook (map center, theme id).
    pub fn ctx(&self) -> &Context {
        self.ctx
    }

    /// Pop the calling component from the stack after the hook
    /// returns. Idempotent — a second call is a no-op. Applied
    /// before `open()`s so `close(); open(c);` replaces this
    /// component with `c`.
    pub fn close(&mut self) {
        self.ops.close = true;
    }

    /// Push `c` on top of the stack after the hook returns. Always
    /// pushes a fresh entry — no identity dedup. A plugin that
    /// wants "open or focus existing" semantics implements that
    /// inside its own `handle_event` (return `close` when already
    /// open, push otherwise).
    pub fn open(&mut self, c: Box<dyn Component>) {
        self.ops.opens.push(c);
    }

    /// Send `msg` onto the App's unified [`AppEvent`] bus, wrapped
    /// as [`AppEvent::Intent`]. The App's main loop drains the bus
    /// in the same iteration's `try_recv` pass after this hook
    /// returns, so `emit + close` still results in the msg being
    /// dispatched (against the post-pop state). A failed send means
    /// the bus receiver has been dropped (App teardown) — silently
    /// ignored.
    pub fn emit(&mut self, msg: UserIntent) {
        self.ops.did_emit = true;
        let _ = self.event_tx.send(AppEvent::Intent(msg));
    }

    /// Signal "this event isn't mine". With no other op queued,
    /// the compositor falls through to the base layer (if this
    /// component isn't already it). If combined with `close` /
    /// `open` / `emit`, the flag is silently dropped — the
    /// component clearly handled the event.
    pub fn ignore(&mut self) {
        self.ops.ignored = true;
    }
}

// ── RenderWindow: draw-time handle ─────────────────────────────────

/// Render-time companion to [`Window`]. Carries the ratatui
/// [`Frame`], the layout area the component may draw into, a
/// read-only snapshot of [`Context`], and — internally, never
/// exposed — the active [`UiTheme`].
///
/// **Components never see `UiTheme`.** They ask for semantic styles
/// via [`StyleKind`] (`Body` / `Muted` / `Accent` / `Highlight` /
/// `Selected` / `Link` / `MutedFg`), and [`RenderWindow::style`]
/// resolves them against the current theme. Adding a new
/// theme-driven field requires one variant + resolver branch, not a
/// signature change in every component.
pub struct RenderWindow<'a, 'b> {
    frame: &'a mut Frame<'b>,
    area: Rect,
    theme: &'a UiTheme,
    #[allow(dead_code)] // read via ctx(); kept even if unused
    ctx: &'a Context,
}

impl<'a, 'b> RenderWindow<'a, 'b> {
    pub(crate) fn new(
        frame: &'a mut Frame<'b>,
        area: Rect,
        theme: &'a UiTheme,
        ctx: &'a Context,
    ) -> Self {
        Self {
            frame,
            area,
            theme,
            ctx,
        }
    }

    /// The area this component is allowed to draw into (usually
    /// the map viewport minus the border).
    pub fn area(&self) -> Rect {
        self.area
    }

    /// App-level snapshot (center, theme id). Kept for parity with
    /// [`Window::ctx`] so `paint_on_map` and future render-side
    /// hooks can read it uniformly.
    #[allow(dead_code)]
    pub fn ctx(&self) -> &Context {
        self.ctx
    }

    /// Clear `rect` and draw a theme-styled bordered panel with
    /// `title` inside it. Returns the inner rect (content region
    /// inside the borders) for further widgets.
    pub fn panel(&mut self, rect: Rect, title: &str) -> Rect {
        let clamped = clamp(rect, self.area);
        self.frame.render_widget(Clear, clamped);
        let block = self.theme.panel(title);
        let inner = block.inner(clamped);
        self.frame.render_widget(block, clamped);
        inner
    }

    /// Draw a `ratatui::widgets::Paragraph` into `rect` (clamped to
    /// the component's area).
    pub fn paragraph(&mut self, p: Paragraph<'static>, rect: Rect) {
        let clamped = clamp(rect, self.area);
        self.frame.render_widget(p, clamped);
    }

    /// Draw a `ratatui::widgets::Table` into `rect`, using `state`
    /// as the selection state.
    pub fn table(&mut self, t: Table<'static>, rect: Rect, state: &mut TableState) {
        let clamped = clamp(rect, self.area);
        self.frame.render_stateful_widget(t, clamped, state);
    }

    /// Draw a vertical scrollbar on the right edge of `rect`. Pass
    /// the panel's *outer* rect — the rail is shrunk vertically by
    /// one row on each side internally so it spans the inner
    /// content area (matching `viewport_length`). The right border
    /// column of the panel is replaced by the rail, which is the
    /// idiomatic ratatui pattern.
    ///
    /// No-op when content fits (`content_length <= viewport_length`):
    /// no reason to draw an indicator that says "you've seen
    /// everything".
    pub fn scrollbar(
        &mut self,
        rect: Rect,
        content_length: u16,
        position: u16,
        viewport_length: u16,
    ) {
        if content_length == 0 || content_length <= viewport_length {
            return;
        }
        let clamped = clamp(rect, self.area);
        // Without this margin the rail height = outer (= inner+2)
        // but `viewport_length` is inner, so the thumb size and
        // position are computed against the wrong rail length.
        // Shrinking by 1 each side aligns rail = viewport.
        let rail = clamped.inner(Margin {
            vertical: 1,
            horizontal: 0,
        });
        if rail.height == 0 {
            return;
        }
        let mut state = ScrollbarState::new(content_length as usize)
            .position(position as usize)
            .viewport_content_length(viewport_length as usize);
        // Disable ▲ / ▼ caps. They eat 2 rows of rail and overlap
        // the panel's border corners ugly-ly. The track itself is
        // a sufficient affordance.
        let bar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        self.frame.render_stateful_widget(bar, rail, &mut state);
    }

    /// Resolve a semantic [`StyleKind`] to a concrete `ratatui::Style`
    /// under the active theme.
    pub fn style(&self, kind: StyleKind) -> Style {
        kind.resolve(self.theme)
    }
}

/// Intersect `rect` with `bounds`, returning the portion inside
/// bounds. If they don't overlap, returns a zero-sized rect
/// (ratatui draws nothing for width or height == 0).
///
/// Uses saturating arithmetic throughout so a malicious or buggy
/// caller passing a `Rect` with huge coordinates (e.g. `Rect::new(
/// u16::MAX, u16::MAX, u16::MAX, u16::MAX)`) cannot overflow u16
/// in the right/bottom computation and wrap into a tiny valid
/// rect that would escape the bounds.
fn clamp(rect: Rect, bounds: Rect) -> Rect {
    let x = rect.x.max(bounds.x);
    let y = rect.y.max(bounds.y);
    let right = rect
        .x
        .saturating_add(rect.width)
        .min(bounds.x.saturating_add(bounds.width));
    let bottom = rect
        .y
        .saturating_add(rect.height)
        .min(bounds.y.saturating_add(bounds.height));
    Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn inside_bounds_returns_same() {
        let bounds = r(0, 0, 100, 100);
        assert_eq!(clamp(r(10, 10, 50, 50), bounds), r(10, 10, 50, 50));
    }

    #[test]
    fn partial_overlap_clipped() {
        let bounds = r(10, 10, 50, 50);
        // rect extends past bounds on the right/bottom
        assert_eq!(clamp(r(30, 30, 100, 100), bounds), r(30, 30, 30, 30));
    }

    #[test]
    fn fully_outside_returns_zero_size() {
        let bounds = r(0, 0, 10, 10);
        let clamped = clamp(r(100, 100, 5, 5), bounds);
        assert_eq!(clamped.width, 0);
        assert_eq!(clamped.height, 0);
    }

    #[test]
    fn huge_coords_dont_overflow_u16() {
        // The overflow-guard case: without saturating_add, this
        // would wrap `rect.x + rect.width` in u16 and produce a
        // small valid right edge, letting the rect escape bounds.
        let bounds = r(0, 0, 100, 100);
        let clamped = clamp(r(u16::MAX - 1, u16::MAX - 1, u16::MAX, u16::MAX), bounds);
        // Fully outside → zero-sized.
        assert_eq!(clamped.width, 0);
        assert_eq!(clamped.height, 0);
    }

    #[test]
    fn zero_sized_input_stays_zero() {
        let bounds = r(0, 0, 100, 100);
        assert_eq!(clamp(r(50, 50, 0, 0), bounds), r(50, 50, 0, 0));
    }

    #[test]
    fn offset_bounds_respected() {
        // bounds not at origin — negative-ish deltas must clip
        // without underflow.
        let bounds = r(20, 20, 50, 50);
        assert_eq!(clamp(r(0, 0, 200, 200), bounds), r(20, 20, 50, 50));
    }
}
