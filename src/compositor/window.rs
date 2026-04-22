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
//!         win.emit(AppMsg::Jump(loc));
//!         win.close();
//!     } else if ev.code == KeyCode::Char('/') {
//!         win.close();
//!         win.open(Box::new(SearchComponent::new()));
//!     }
//! }
//! ```
//!
//! Method calls queue into [`WindowOps`]. The compositor drains the
//! queue after the hook returns and applies the ops atomically in a
//! deterministic order: `close` → `opens` (with TypeId dedup) → and
//! the collected `msgs` are returned to `App::dispatch`.
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

use ratatui::Frame;
use ratatui::layout::Rect as RRect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Clear, StatefulWidget, Widget};

use crate::app::AppMsg;
use crate::compositor::{Component, Context};
use crate::theme::UiTheme;
use crate::widget;

/// Queue of actions a [`Component`] hook recorded through [`Window`].
/// Drained and applied by the compositor after the hook returns.
#[derive(Default)]
pub(crate) struct WindowOps {
    /// `true` if the plugin called [`Window::close`]. Pops the
    /// calling component. Applied before `opens` so `close + open`
    /// replaces the component in the stack slot.
    pub close: bool,
    /// Components queued by [`Window::open`]. Each goes through
    /// TypeId dedup when pushed; a duplicate of an existing stack
    /// entry shifts focus to the existing one instead.
    pub opens: Vec<Box<dyn Component>>,
    /// Messages for [`App::dispatch`](crate::app::App). Returned
    /// from `Compositor::handle_event` to the caller (App), which
    /// dispatches them after the ops have been applied.
    pub msgs: Vec<AppMsg>,
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
        !self.close && self.opens.is_empty() && self.msgs.is_empty()
    }
}

/// Handle components receive on every hook. Constrained by design:
/// no `&mut Compositor` is reachable through it, so components
/// cannot break focus / stack invariants even if buggy.
///
/// Read-only accessors (`ctx`) give the component what it needs for
/// decision-making without granting any capability.
pub struct Window<'a> {
    ops: &'a mut WindowOps,
    ctx: &'a Context,
}

impl<'a> Window<'a> {
    pub(crate) fn new(ops: &'a mut WindowOps, ctx: &'a Context) -> Self {
        Self { ops, ctx }
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

    /// Push `c` on top of the stack after the hook returns. Subject
    /// to TypeId dedup: if a component of the same concrete type is
    /// already on the stack, focus shifts to the existing instance
    /// and `c` is dropped.
    pub fn open(&mut self, c: Box<dyn Component>) {
        self.ops.opens.push(c);
    }

    /// Queue `msg` for `App::dispatch`. Dispatched by the caller
    /// (App) after the compositor has applied `close` / `open`. For
    /// typical `emit + close` patterns this means the msg still
    /// fires, but the component is already popped when it runs —
    /// identical to the old `EventResult::Close(msgs)` semantic.
    pub fn emit(&mut self, msg: AppMsg) {
        self.ops.msgs.push(msg);
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
/// (body / muted / accent / highlight / selected / link), and
/// [`RenderWindow`] maps them to the current theme's concrete
/// `Style`. Adding a new theme-driven field requires one accessor
/// here, not a signature change in every plugin.
///
/// `frame()` remains as an escape hatch for widgets not yet wrapped
/// by this module (lists, tables, scrollable paragraphs). A
/// follow-up refactor will fold those into Window primitives and
/// retire the escape hatch — at which point plugins won't need
/// `use ratatui::*` at all.
pub struct RenderWindow<'a, 'b> {
    frame: &'a mut Frame<'b>,
    area: RRect,
    theme: &'a UiTheme,
    #[allow(dead_code)] // read via ctx(); kept even if unused
    ctx: &'a Context,
}

impl<'a, 'b> RenderWindow<'a, 'b> {
    pub(crate) fn new(
        frame: &'a mut Frame<'b>,
        area: RRect,
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
    pub fn area(&self) -> widget::Rect {
        self.area.into()
    }

    /// App-level snapshot (center, theme id). Kept for parity with
    /// [`Window::ctx`] so `paint_on_map` and future render-side
    /// hooks can read it uniformly.
    #[allow(dead_code)]
    pub fn ctx(&self) -> &Context {
        self.ctx
    }

    /// Render a ratatui widget into `rect`. `rect` is **clamped to
    /// `self.area()`** before drawing, so a component cannot paint
    /// outside the area the compositor allocated to it — the map
    /// border, footer, and sibling components are all protected.
    ///
    /// This is the only way for a component to draw; direct access
    /// to the underlying `Frame` is not exposed.
    ///
    /// Accepts anything convertible to [`widget::Rect`] (including
    /// `ratatui::layout::Rect`) as a bridge during the plugin
    /// migration — remove the `impl Into` once C3 lands.
    pub fn render_widget<W: Widget>(&mut self, widget: W, rect: impl Into<widget::Rect>) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        self.frame.render_widget(widget, clamped);
    }

    /// Stateful counterpart to [`render_widget`] — for widgets like
    /// `List` / `Table` that keep per-frame `*State`. Same rect
    /// clamping.
    pub fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        rect: impl Into<widget::Rect>,
        state: &mut W::State,
    ) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        self.frame.render_stateful_widget(widget, clamped, state);
    }

    /// Clear the cells in `rect` (rect-clamped). Useful before
    /// drawing a popup so whatever was underneath doesn't bleed
    /// through.
    pub fn clear(&mut self, rect: impl Into<widget::Rect>) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        self.frame.render_widget(Clear, clamped);
    }

    /// Clear `rect` and draw a theme-styled bordered panel with
    /// `title` inside it. Returns the inner rect (content region
    /// inside the borders) for further widgets.
    pub fn panel(&mut self, rect: impl Into<widget::Rect>, title: &str) -> widget::Rect {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        self.frame.render_widget(Clear, clamped);
        let block = self.theme.panel(title);
        let inner = block.inner(clamped);
        self.frame.render_widget(block, clamped);
        inner.into()
    }

    /// Build a theme-styled [`Block`] without drawing anything. Use
    /// when the content widget (Paragraph / List) needs to own the
    /// Block via `.block(...)`, as in wiki's scrollable list or
    /// help's centered text overlay.
    pub fn panel_block<'t>(&self, title: &'t str) -> Block<'t> {
        self.theme.panel(title)
    }

    // ── Semantic style accessors (UiTheme hidden) ─────────────────

    /// Plain body text style. Maps to the theme's "fg on bg"
    /// combination; plugin never sees which palette entry that is.
    pub fn body_style(&self) -> Style {
        self.theme.text()
    }

    /// Subdued text — hints, distances, coordinates, auxiliary
    /// info. Lower contrast than body.
    pub fn muted_style(&self) -> Style {
        self.theme.muted()
    }

    /// Primary accent — section titles, key hints in help, plugin
    /// panel headers.
    pub fn accent_style(&self) -> Style {
        self.theme.accent_style()
    }

    /// Secondary accent — the "look at this one" highlight used for
    /// selected wiki titles. Distinct from [`selected_style`]
    /// (which is the full selected-row chrome including bold);
    /// this is just the alt accent colour on fg.
    pub fn highlight_style(&self) -> Style {
        Style::default().fg(self.theme.accent_alt)
    }

    /// Selected list / table row — accent colour + bold. Matches
    /// the palette's row highlight and search candidate selection.
    pub fn selected_style(&self) -> Style {
        self.theme.selected()
    }

    /// URL / clickable text. Terminals that detect OSC 8 or auto-
    /// link by regex will activate it. Distinct from plain accent
    /// because it's underlined.
    pub fn link_style(&self) -> Style {
        self.theme.link()
    }

    /// Foreground-only style using the muted colour — suitable for
    /// thin separator lines (`─`) where `muted_style()`'s
    /// foreground-on-background combination would bleed the bg.
    pub fn muted_fg_style(&self) -> Style {
        Style::default().fg(self.theme.muted_color)
    }

    // ── Span constructors (compose `Line`s from styled text) ──────

    /// Body-styled text span. Pair with [`Line::from`] /
    /// [`Line::from(vec![..])`] to build multi-span lines without
    /// importing `Style` / `Span::styled` from ratatui.
    pub fn span_body<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.body_style())
    }

    /// Muted-styled text span.
    pub fn span_muted<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.muted_style())
    }

    /// Accent-styled text span (primary accent).
    pub fn span_accent<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.accent_style())
    }

    /// Highlight-styled text span (secondary accent, e.g. selected
    /// wiki title).
    pub fn span_highlight<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.highlight_style())
    }

    /// Link-styled text span (underlined, alt accent).
    pub fn span_link<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.link_style())
    }

    /// Foreground-only muted span — separator glyphs etc.
    pub fn span_separator<'t, T: Into<std::borrow::Cow<'t, str>>>(&self, text: T) -> Span<'t> {
        Span::styled(text, self.muted_fg_style())
    }

    // ── New widget-descriptor API ────────────────────────────────────
    //
    // These accept `widget::*` descriptors and convert to ratatui
    // internally. Plugins use these in place of `render_widget`
    // after C3 migration. `allow(dead_code)` removed when C3 lands.

    /// Draw a [`widget::Paragraph`] descriptor into `rect`. The
    /// paragraph's optional `framed_title` is expanded into a
    /// theme-styled bordered block at render time.
    #[allow(dead_code)]
    pub fn paragraph(&mut self, p: widget::Paragraph, rect: impl Into<widget::Rect>) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        let r = p.into_ratatui(self.theme);
        self.frame.render_widget(r, clamped);
    }

    /// Draw a [`widget::List`] descriptor into `rect`.
    #[allow(dead_code)]
    pub fn list(&mut self, l: widget::List, rect: impl Into<widget::Rect>) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        let r: ratatui::widgets::List = l.into();
        self.frame.render_widget(r, clamped);
    }

    /// Draw a [`widget::Table`] descriptor into `rect`, using `sel`
    /// as the selection state.
    #[allow(dead_code)]
    pub fn table(
        &mut self,
        t: widget::Table,
        rect: impl Into<widget::Rect>,
        sel: &widget::TableSel,
    ) {
        let w_rect: widget::Rect = rect.into();
        let clamped = clamp(w_rect.into(), self.area);
        let r: ratatui::widgets::Table = t.into();
        let mut state: ratatui::widgets::TableState = (*sel).into();
        self.frame.render_stateful_widget(r, clamped, &mut state);
    }

    /// Resolve a semantic [`widget::StyleKind`] to a concrete
    /// [`widget::TextStyle`] under the active theme.
    #[allow(dead_code)]
    pub fn style(&self, kind: widget::StyleKind) -> widget::TextStyle {
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
fn clamp(rect: RRect, bounds: RRect) -> RRect {
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
    RRect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> RRect {
        RRect::new(x, y, w, h)
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
