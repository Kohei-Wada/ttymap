//! [`LuaCardComponent`] — a focused [`Component`] pushed onto the
//! compositor stack by `ttymap.api.card.open(spec)` (A3).
//!
//! Spec table fields (all optional):
//! - `name = "..."` — display label shown in the focused-footer chip
//! - `render = function() return lines end` — panel body
//! - `handle_key = function(key) return action end` — focused keys
//! - `footer_hints = { {key, label}, ... }` — focused footer hints
//!
//! All `LuaCardComponent`s render in the left sidebar — there's no
//! free-floating / anchored layout for plugin-defined panels. The
//! `spec.layout` field is no longer read; existing scripts that set
//! it just see their setting silently ignored.
//!
//! **No map-paint or `poll` / `loop` keys** — per-frame work belongs
//! on a `ttymap.api.frame.on_tick(fn)` (or `ttymap.on_event`)
//! subscription. A card opened via `card.open` does focused-UI work
//! only; map paint and async drain run in the per-frame tick on the
//! main thread.
//!
//! Lifetime: the matching [`CardHandle`](super::card_handle::CardHandle)
//! (returned to Lua by `card.open`) holds the same
//! [`CardId`](crate::compositor::CardId) reserved at the
//! call site. Lua-side `handle:close()` enqueues an
//! [`Op::Close`](crate::compositor::op::Op::Close) onto the shared
//! [`OpsBuffer`](crate::compositor::op::OpsBuffer); the App applies it per
//! iteration via
//! [`crate::compositor::Compositor::close_by_id`].
//! Idempotent — repeated `close()` calls just enqueue duplicate
//! `Op::Close` entries that are no-ops once the component is gone.
//!
//! Drain plumbing (`ttymap.map:jump`, `ttymap.api.frame.export`)
//! lives in the **shared Lua state** — *not* on this per-window
//! component. The shared cells are returned by
//! [`crate::lua::api::install`] inside [`LuaHostHandles`] (one set
//! for the whole subsystem) and drained centrally by `App` per
//! frame. `card.open` runs in the shared Lua VM, so its callbacks'
//! `ttymap.map:jump(...)` calls hit the same shared senders every
//! other plugin uses.
//!
//! Per audit §13: errors are logged and recovered, never propagated.
//! A buggy plugin must not take the host down.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{Lua, Table};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::card_parse::{
    KeyAction, key_code_to_lua, parse_footer_hints, parse_item_value, parse_line_value,
};
use super::handle::{CallOutcome, LuaBridgeHandle};
use crate::compositor::Component;
use crate::compositor::window::{RenderWindow, Window};
use crate::theme::StyleKind;

// ── Component ──────────────────────────────────────────────────────

/// A [`Component`] backed by a Lua spec table. Pushed onto the
/// compositor stack by `ttymap.api.card.open(spec)`; popped when
/// the matching [`CardHandle`](super::card_handle::CardHandle)
/// enqueues an [`Op::Close`](crate::compositor::op::Op::Close) keyed by the
/// reserved [`CardId`](crate::compositor::CardId), or when
/// the spec's `handle_key` returns `{ close = true }`.
pub struct LuaCardComponent {
    /// Bridge plumbing — fresh `Lua` VM, registered spec table,
    /// log tag (= identification used in warnings).
    handle: LuaBridgeHandle,
    /// User-facing display label, read from `spec.name` if present
    /// at construction. Falls back to the handle's log tag (the
    /// `chunk_name` passed in by `card.open`). Leaked once so
    /// [`Component::name`] can satisfy the `&'static str` signature;
    /// bounded cost since `LuaCardComponent` is rebuilt at most a
    /// few times per program lifetime.
    display: &'static str,
    /// Whether the spec exposes a `render` function. Marker-only
    /// windows (no panel UI) omit it; without this flag the adapter
    /// would still paint an empty framed Paragraph over the map.
    has_render: bool,
    /// Whether the spec exposes an `items` function (list mode).
    /// When true *and* `items()` returns a non-empty list, the
    /// component renders as a ratatui `List` with native
    /// selection / scroll state instead of the free-form
    /// Paragraph path. Empty `items()` falls through to `render`
    /// so plugins can use that for "Loading..." placeholders.
    has_items: bool,
    /// Static footer hints from `spec.footer_hints`. Read once at
    /// construction so [`Component::footer_hints`] can hand back
    /// `&'static str` without leaking per call. Empty when the spec
    /// omits the field.
    footer_hints: Vec<(&'static str, &'static str)>,
    /// First visible line index when the rendered content overflows
    /// the slot. Bridge-managed via the section scroll keys
    /// (PageUp / PageDown / C-n / C-p / Up / Down / Home / End).
    /// `Cell` because `render` takes `&self`. Used by the Paragraph
    /// path only — list-mode scrolling lives in `list_state.offset`.
    scroll_offset: std::cell::Cell<u16>,
    /// Last rendered inner height of this section's panel (i.e.
    /// `area - frame border`). Cached so PageUp / PageDown / C-d /
    /// C-u in `handle_key` can use the *current* slot height as
    /// the page step size — `handle_key` itself has no Rect.
    /// Defaults to a sane fallback (10) when no frame has rendered
    /// yet.
    last_inner_height: std::cell::Cell<u16>,
    /// Persistent ratatui `ListState` for the list-mode render
    /// path. ratatui mutates `state.offset` in `render` to keep
    /// `selected` in view; persisting the state across frames lets
    /// that book-keeping survive. `RefCell` because `render` takes
    /// `&self`.
    list_state: std::cell::RefCell<ListState>,
}

impl LuaCardComponent {
    /// Build a `LuaCardComponent` from a spec table evaluated in
    /// `lua`. The spec is everything `card.open` was passed; the
    /// caller has already extracted the close flag and the lua state
    /// it lives in.
    ///
    /// `log_tag` is the identifier used in `log::warn!` messages
    /// (`lua[<log_tag>]: render() failed: …`) and as the fallback
    /// for [`Component::name`] when `spec.name` is missing.
    ///
    /// The shared Lua state owns the senders for jump / zoom /
    /// fly_to / frame.export (lowered to `Op::Command` on the shared
    /// `OpsBuffer`) and the host-shared `center` / `zoom` mutexes;
    /// this component does not drain them (App drains them
    /// centrally per loop iteration).
    pub fn from_spec(lua: Lua, spec: Table, log_tag: &'static str) -> mlua::Result<Self> {
        // Display name: spec's `name` if set, else the log tag.
        // Leak once; bounded by the number of windows opened.
        let display: &'static str = spec
            .get::<String>("name")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
            .unwrap_or(log_tag);
        let has_render = matches!(
            spec.get::<mlua::Value>("render"),
            Ok(mlua::Value::Function(_))
        );
        let has_items = matches!(
            spec.get::<mlua::Value>("items"),
            Ok(mlua::Value::Function(_))
        );
        let footer_hints = parse_footer_hints(&spec);
        let handle = LuaBridgeHandle::new(lua, spec, log_tag)?;
        Ok(Self {
            handle,
            display,
            has_render,
            has_items,
            footer_hints,
            scroll_offset: std::cell::Cell::new(0),
            last_inner_height: std::cell::Cell::new(10),
            list_state: std::cell::RefCell::new(ListState::default()),
        })
    }

    /// Pull the `render()` lines from the Lua spec as raw line
    /// descriptors. Each line is a vec of `(text, style_kind)` spans.
    /// Returns an empty vec on any error (with a warning logged).
    ///
    /// Supported per-line shapes:
    /// - **string** → single Body span: `"hello"`
    /// - **array of `{text, style}` records** — multi-span line.
    fn render_lines(&self) -> Vec<Vec<(String, StyleKind)>> {
        match self.handle.try_call::<_, Vec<mlua::Value>>("render", ()) {
            CallOutcome::Ok(raw) => raw.into_iter().map(parse_line_value).collect(),
            CallOutcome::Missing | CallOutcome::Errored => Vec::new(),
        }
    }

    /// Pull the `items()` list from the Lua spec. Each item is a
    /// vec of lines (each line a vec of spans), so a 2-line list
    /// item like quake's "M5.7 Tokyo / 2h ago" comes back as a
    /// `Vec<Vec<Span>>` of length 2. Empty list on missing /
    /// errored / non-function spec field.
    fn render_items(&self) -> Vec<Vec<Vec<(String, StyleKind)>>> {
        match self.handle.try_call::<_, Vec<mlua::Value>>("items", ()) {
            CallOutcome::Ok(raw) => raw.into_iter().map(parse_item_value).collect(),
            CallOutcome::Missing | CallOutcome::Errored => Vec::new(),
        }
    }

    /// Read the current `selected()` index from the spec. Lua side
    /// is 1-based; we convert to ratatui's 0-based here. Out-of-
    /// range or non-numeric values become `None` (no selection).
    fn selected_index(&self, item_count: usize) -> Option<usize> {
        match self.handle.try_call::<_, mlua::Value>("selected", ()) {
            CallOutcome::Ok(mlua::Value::Integer(i)) => {
                if i < 1 {
                    return None;
                }
                let zero = (i - 1) as usize;
                (zero < item_count).then_some(zero)
            }
            _ => None,
        }
    }

    /// Run the Lua side of `handle_key` and return the host action
    /// the script asked for.
    ///
    /// Three outcomes:
    /// - **No `handle_key` field** → `KeyAction::Ignore`. Mirrors
    ///   the Component trait's default impl: the spec opts out of
    ///   keymap consumption and the event flows to the base layer.
    /// - **Lua returned `nil`** → `KeyAction::Consume`. The handler
    ///   ran, decided "this isn't mine", but doesn't want the base
    ///   layer to see it either.
    /// - **Lua returned a table** → its `close` / `ignore` flags map
    ///   directly to `Window::close` / `Window::ignore`. Anything
    ///   else (including a malformed table or a runtime error) logs
    ///   a warning and falls back to `Consume` so a buggy plugin
    ///   can't accidentally leak its keys to the rest of the app.
    fn dispatch_key(&self, event: KeyEvent) -> KeyAction {
        let key = match self.build_key_table(event) {
            Ok(k) => k,
            Err(e) => {
                log::warn!(
                    "lua[{}]: build_key_table failed: {}",
                    self.handle.log_tag(),
                    e
                );
                return KeyAction::Consume;
            }
        };
        match self.handle.try_call::<_, mlua::Value>("handle_key", key) {
            CallOutcome::Ok(ret) => KeyAction::from_lua_return(ret),
            CallOutcome::Missing => KeyAction::Ignore,
            CallOutcome::Errored => KeyAction::Consume,
        }
    }

    /// Render the spec's `render()` output as a free-form
    /// Paragraph. Used for help-style content and as the
    /// empty-state fallback when a list-driven plugin returns no
    /// items yet.
    fn render_paragraph(
        &self,
        win: &mut RenderWindow,
        outer: ratatui::layout::Rect,
        inner: ratatui::layout::Rect,
    ) {
        let body = win.style(StyleKind::Body);
        let raw_lines = self.render_lines();
        let total_lines = raw_lines.len() as u16;

        let lines: Vec<Line<'static>> = raw_lines
            .into_iter()
            .map(|spans| {
                let rendered: Vec<Span<'static>> = spans
                    .into_iter()
                    .map(|(text, kind)| Span::styled(text, win.style(kind)))
                    .collect();
                Line::from(rendered)
            })
            .collect();

        let mut offset = self.scroll_offset.get();
        // Clamp against the freshly-rendered line count so PageDown
        // overshoot self-corrects here (and the offset never traps
        // the section in blank space).
        let max_offset = total_lines.saturating_sub(inner.height);
        if offset > max_offset {
            offset = max_offset;
        }
        self.scroll_offset.set(offset);

        // Wrap long lines at the inner width instead of truncating —
        // travel-plugin stop notes, wiki article bodies, etc. routinely
        // exceed the sidebar's column count. `trim: false` preserves
        // intentional leading whitespace (e.g. "  ▶ 1. Tokyo" becomes
        // "  Tokyo — neon nights ..." indentation on overflow rows).
        // Caveat: with wrap on, the scrollbar's "total" count
        // undercounts because we feed it input-line count, not the
        // post-wrap render count. ratatui's Paragraph doesn't expose
        // the latter; the scrollbar position is still directionally
        // right (offset increases toward the bottom), just slightly
        // off in scale. Acceptable for sidebar use.
        let paragraph = Paragraph::new(lines)
            .style(body)
            .scroll((offset, 0))
            .wrap(Wrap { trim: false });
        win.paragraph(paragraph, inner);
        win.scrollbar(outer, total_lines, offset, inner.height);
    }

    /// Render the spec's `items()` output as a stateful
    /// `ratatui::List`. ratatui handles per-frame
    /// scroll-to-selected via `ListState.offset`; we just feed it
    /// the current selection (1-based from Lua → 0-based here)
    /// and persist the state across frames so the offset survives.
    fn render_list(
        &self,
        win: &mut RenderWindow,
        outer: ratatui::layout::Rect,
        inner: ratatui::layout::Rect,
        raw_items: Vec<Vec<Vec<(String, StyleKind)>>>,
    ) {
        let body = win.style(StyleKind::Body);
        let highlight_style = win.style(StyleKind::Highlight);

        // Materialise items as ratatui ListItems. Each item is a
        // `Vec<Line>` (1+ lines) so a quake-style "M5.7 Tokyo /
        // 2h ago" lands as a 2-line ListItem.
        //
        // Each input line is run through `wrap_styled_line` first
        // so long content (travel-plugin route summaries, wiki
        // article titles, …) wraps at the inner width instead of
        // truncating at the right edge. ratatui's `List` natively
        // supports multi-line items, so a single Lua-side line that
        // wraps to two display rows just makes the ListItem two
        // rows tall. Selection still picks the whole logical item.
        let list_items: Vec<ListItem<'static>> = raw_items
            .iter()
            .map(|item_lines| {
                let lines: Vec<Line<'static>> = item_lines
                    .iter()
                    .flat_map(|spans| {
                        wrap_styled_line(spans, inner.width)
                            .into_iter()
                            .map(|wrapped_spans| {
                                let rendered: Vec<Span<'static>> = wrapped_spans
                                    .into_iter()
                                    .map(|(text, kind)| Span::styled(text, win.style(kind)))
                                    .collect();
                                Line::from(rendered)
                            })
                    })
                    .collect();
                ListItem::new(lines)
            })
            .collect();

        let total = raw_items.len();
        let selected = self.selected_index(total);

        // Update the persistent state. ratatui's `render` mutates
        // `state.offset` to keep `state.selected()` in view; we
        // hand it a `&mut` view of the cell-stored state.
        let mut state = self.list_state.borrow_mut();
        state.select(selected);

        let list = List::new(list_items)
            .style(body)
            .highlight_style(highlight_style);
        win.list(list, inner, &mut state);

        // Scrollbar driven by ratatui's own scroll bookkeeping.
        // Approximate: rail length = inner.height (one row per
        // item is the simple case; multi-line items shrink the
        // ratio but the indicator still tells the right story).
        let viewport_items = total.min(inner.height as usize) as u16;
        win.scrollbar(outer, total as u16, state.offset() as u16, viewport_items);
    }

    fn build_key_table(&self, event: KeyEvent) -> mlua::Result<Table> {
        let table = self.handle.lua().create_table()?;
        let (code, ch) = key_code_to_lua(event.code);
        table.set("code", code)?;
        if let Some(c) = ch {
            // `char` is set only for printable Char(c) events; the
            // Lua side reads it as `key.char` when `key.code ==
            // "Char"`. Other key codes leave it unset / nil.
            table.set("char", c.to_string())?;
        }
        table.set("ctrl", event.modifiers.contains(KeyModifiers::CONTROL))?;
        table.set("shift", event.modifiers.contains(KeyModifiers::SHIFT))?;
        table.set("alt", event.modifiers.contains(KeyModifiers::ALT))?;
        Ok(table)
    }
}

impl Component for LuaCardComponent {
    fn handle_key(&mut self, event: KeyEvent, win: &mut Window) {
        let action = self.dispatch_key(event);

        // When the Lua spec didn't consume the event, the bridge
        // applies built-in scroll keys so overflow content is
        // reachable without every plugin re-implementing it.
        // Plugins that *want* one of these (e.g. aircraft uses
        // Up / Down to pick a row) consume by returning nil —
        // those never reach this branch.
        //
        // j / k and C-u / C-d stay untouched here and pass through
        // to the base layer (map pan / half-page pan). Letting them
        // scroll the focused section instead would steal navigation
        // keys the user is in the middle of using; the dedicated
        // PageUp / PageDown / C-n / C-p / Home / End cover the
        // intra-section case without that ambiguity.
        if action == KeyAction::Ignore {
            let cur = self.scroll_offset.get();
            let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
            let page = self.last_inner_height.get().max(1);
            let next = match (event.code, ctrl) {
                (KeyCode::Down, false) => Some(cur.saturating_add(1)),
                (KeyCode::Up, false) => Some(cur.saturating_sub(1)),
                (KeyCode::Char('n'), true) => Some(cur.saturating_add(1)),
                (KeyCode::Char('p'), true) => Some(cur.saturating_sub(1)),
                (KeyCode::PageDown, _) => Some(cur.saturating_add(page)),
                (KeyCode::PageUp, _) => Some(cur.saturating_sub(page)),
                (KeyCode::Home, _) => Some(0),
                (KeyCode::End, _) => Some(u16::MAX),
                _ => None,
            };
            if let Some(v) = next {
                self.scroll_offset.set(v);
                return; // consumed
            }
        }

        // Host-side jump / frame.export the callback queued hits the
        // shared `OpsBuffer`, not per-window receivers. App drains
        // it centrally each frame.
        match action {
            KeyAction::Close => win.close(),
            KeyAction::Ignore => win.ignore(),
            KeyAction::Consume => {}
        }
    }

    fn render(&self, win: &mut RenderWindow) {
        if !self.has_render && !self.has_items {
            // Marker-only / map-only cards opt out of side-area
            // chrome; without this guard we'd paint an empty framed
            // Paragraph over the map.
            return;
        }
        // Sidebar sections fill the entire allocated slot — the
        // sidebar layout already picked the rect, no inner anchor
        // or width logic needed.
        let area = win.area();
        let inner = win.panel(area, self.display);
        // Snapshot the slot's content height so handle_key can use
        // it as the page step for PageUp / PageDown / C-d / C-u
        // without computing layout itself.
        self.last_inner_height.set(inner.height);

        // List path: when `items` is set and returns at least one
        // entry, render as a ratatui `List`. Otherwise (items is
        // missing OR returned empty), fall through to the
        // paragraph path which renders `render()`. This makes
        // `render` a natural empty-state placeholder for plugins
        // that drive a list — quake's "feed off" / "loading"
        // messages, for example.
        if self.has_items {
            let raw_items = self.render_items();
            if !raw_items.is_empty() {
                self.render_list(win, area, inner, raw_items);
                return;
            }
        }

        if self.has_render {
            self.render_paragraph(win, area, inner);
        }
    }

    fn name(&self) -> &'static str {
        self.display
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.footer_hints.clone()
    }

    fn placement(&self) -> crate::compositor::Placement {
        crate::compositor::Placement::Sidebar
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Word-wrap a single styled line so it fits inside `max_width`
/// terminal cells, returning one or more output rows. Each row is
/// itself a list of styled spans — the wrap walks word by word and
/// merges adjacent same-style runs back together so multi-style
/// lines (e.g. "Japan · Golden Route   10-14 days") preserve their
/// per-segment colours across the break.
///
/// Whitespace boundaries drive the break (`split_inclusive` keeps
/// the trailing space on each word so the right edge doesn't end on
/// a leftover blank). Words wider than `max_width` themselves stay
/// intact on a single row — better mid-overflow than mid-word splits
/// that destroy CJK characters or compound words. ratatui will
/// truncate the overflow at the cell boundary.
///
/// Fast path: when the line already fits, returns `[input.to_vec()]`
/// without copying spans.
fn wrap_styled_line(
    spans: &[(String, StyleKind)],
    max_width: u16,
) -> Vec<Vec<(String, StyleKind)>> {
    if max_width == 0 {
        return vec![spans.to_vec()];
    }
    let max_w = max_width as usize;
    let total: usize = spans.iter().map(|(s, _)| s.as_str().width()).sum();
    if total <= max_w {
        return vec![spans.to_vec()];
    }

    let mut out: Vec<Vec<(String, StyleKind)>> = vec![Vec::new()];
    let mut row_width = 0usize;

    for (text, style) in spans {
        for word in text.split_inclusive(char::is_whitespace) {
            let w = word.width();
            // If the word doesn't fit on the current row AND the row
            // already has content, start a new row. Empty rows accept
            // any word — including ones wider than `max_w` — so we
            // never loop forever on an oversized token.
            if row_width + w > max_w && row_width > 0 {
                out.push(Vec::new());
                row_width = 0;
            }
            let row = out.last_mut().expect("at least one row by construction");
            // Merge into the last span if same style — keeps the
            // output as compact as the input intended.
            if let Some(last) = row.last_mut() {
                if last.1 == *style {
                    last.0.push_str(word);
                } else {
                    row.push((word.to_string(), *style));
                }
            } else {
                row.push((word.to_string(), *style));
            }
            row_width += w;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal helper: build a `LuaCardComponent` from a Lua source
    /// snippet that returns the spec table directly. `card.open`
    /// gets its spec the same way — caller-side `eval`, resulting
    /// Table handed in. Bypasses the whole `register_*` dance because
    /// these tests exercise component behaviour, not registration.
    fn make(source: &str, log_tag: &'static str) -> LuaCardComponent {
        let lua = mlua::Lua::new();
        let spec: Table = lua.load(source).eval().expect("eval spec");
        LuaCardComponent::from_spec(lua, spec, log_tag).expect("from_spec")
    }

    #[test]
    fn display_picks_up_spec_name() {
        let c = make(
            r#"return { name = "International Space Station", render = function() return {} end }"#,
            "iss",
        );
        assert_eq!(c.name(), "International Space Station");
    }

    #[test]
    fn display_falls_back_to_log_tag_when_spec_omits_name() {
        let c = make(r#"return {}"#, "anon");
        assert_eq!(c.name(), "anon");
    }

    #[test]
    fn render_lines_round_trip_through_lua() {
        let c = make(
            r#"return {
                name = "demo",
                render = function() return { "alpha", "beta", "gamma" } end,
            }"#,
            "demo",
        );
        let lines = c.render_lines();
        let texts: Vec<&str> = lines.iter().map(|spans| spans[0].0.as_str()).collect();
        assert_eq!(texts, vec!["alpha", "beta", "gamma"]);
        assert!(lines.iter().all(|spans| spans[0].1 == StyleKind::Body));
    }

    #[test]
    fn render_lines_recovers_when_lua_throws() {
        let c = make(
            r#"return {
                name = "broken",
                render = function() error("kaboom") end,
            }"#,
            "broken",
        );
        // Should not panic — error is logged, we get an empty result.
        assert!(c.render_lines().is_empty());
    }

    #[test]
    fn render_lines_recovers_when_field_is_missing() {
        let c = make(r#"return { name = "noop" }"#, "noop");
        assert!(c.render_lines().is_empty());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn missing_handler_dispatches_to_ignore() {
        let c = make(r#"return { name = "noop" }"#, "noop");
        assert_eq!(c.dispatch_key(key(KeyCode::Esc)), KeyAction::Ignore);
    }

    #[test]
    fn handler_returning_nil_consumes() {
        let c = make(
            r#"return {
                name = "modal",
                handle_key = function(_) return nil end,
            }"#,
            "modal",
        );
        assert_eq!(c.dispatch_key(key(KeyCode::Char('a'))), KeyAction::Consume);
    }

    #[test]
    fn handler_returning_close_table_closes() {
        let c = make(
            r#"return {
                name = "esc",
                handle_key = function(k)
                    if k.code == "Esc" then return { close = true } end
                    return nil
                end,
            }"#,
            "esc",
        );
        assert_eq!(c.dispatch_key(key(KeyCode::Esc)), KeyAction::Close);
        assert_eq!(c.dispatch_key(key(KeyCode::Char('x'))), KeyAction::Consume);
    }

    #[test]
    fn handler_returning_ignore_table_ignores() {
        let c = make(
            r#"return {
                name = "passthrough",
                handle_key = function(_) return { ignore = true } end,
            }"#,
            "passthrough",
        );
        assert_eq!(c.dispatch_key(key(KeyCode::Char('q'))), KeyAction::Ignore);
    }

    #[test]
    fn handler_runtime_error_falls_back_to_consume() {
        let c = make(
            r#"return {
                name = "broken",
                handle_key = function(_) error("kaboom") end,
            }"#,
            "broken",
        );
        assert_eq!(c.dispatch_key(key(KeyCode::Char('a'))), KeyAction::Consume);
    }

    #[test]
    fn handler_returning_unknown_value_consumes() {
        let c = make(
            r#"return {
                name = "weird",
                handle_key = function(_) return "yolo" end,
            }"#,
            "weird",
        );
        assert_eq!(c.dispatch_key(key(KeyCode::Char('z'))), KeyAction::Consume);
    }

    // Close coverage moved to `card_handle::tests::close_enqueues_op_close_idempotent`:
    // the close path no longer goes through Component::poll, so the
    // CloseFlag polling tests retire with the flag itself.

    // ── wrap_styled_line ─────────────────────────────────────────

    fn span(text: &str, kind: StyleKind) -> (String, StyleKind) {
        (text.to_string(), kind)
    }

    #[test]
    fn wrap_styled_line_passes_through_when_input_fits() {
        let line = vec![span("short", StyleKind::Body)];
        let out = wrap_styled_line(&line, 80);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], line);
    }

    #[test]
    fn wrap_styled_line_breaks_at_word_boundary() {
        // "alpha beta gamma" = "alpha "(6) + "beta "(5) + "gamma"(5)
        // = 16 cells. With max=10:
        //   row 1: "alpha "  (6, next "beta " would push to 11 > 10)
        //   row 2: "beta gamma"  (10, ≤ 10)
        let line = vec![span("alpha beta gamma", StyleKind::Body)];
        let out = wrap_styled_line(&line, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0][0].0, "alpha ");
        assert_eq!(out[1][0].0, "beta gamma");
    }

    #[test]
    fn wrap_styled_line_preserves_per_segment_styles_across_break() {
        // "Japan · " (muted) "Golden Route" (accent) "  " (body)
        // "10-14 days" (muted). Total ≈ 32 cells; with max=20 we
        // expect a break between "Route" and "10-14".
        let line = vec![
            span("Japan ", StyleKind::Muted),
            span("Route ", StyleKind::Accent),
            span("days", StyleKind::Muted),
        ];
        let out = wrap_styled_line(&line, 11);
        assert!(out.len() >= 2, "should wrap into at least 2 rows");
        // Each output span must carry its original style — no style
        // collapse to a single colour.
        let kinds_present: Vec<StyleKind> = out
            .iter()
            .flat_map(|row| row.iter().map(|(_, k)| *k))
            .collect();
        assert!(kinds_present.iter().any(|k| *k == StyleKind::Muted));
        assert!(kinds_present.iter().any(|k| *k == StyleKind::Accent));
    }

    #[test]
    fn wrap_styled_line_keeps_oversized_word_on_one_row() {
        // "supercalifragilisticexpialidocious" = 34 cells; max 10
        // can't break it (no internal spaces) so it stays whole and
        // overflows. Better than mid-word splitting which would
        // break CJK / acronyms.
        let line = vec![span("supercalifragilisticexpialidocious", StyleKind::Body)];
        let out = wrap_styled_line(&line, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0][0].0, "supercalifragilisticexpialidocious");
    }

    #[test]
    fn wrap_styled_line_merges_adjacent_same_style_spans_within_wrapped_row() {
        // After wrapping, adjacent same-style words should land
        // back as one span in each output row (compactness — and
        // it lets ratatui draw them with one styled write).
        let line = vec![
            span("alpha ", StyleKind::Body),
            span("beta ", StyleKind::Body),
            span("gamma ", StyleKind::Body),
            span("delta", StyleKind::Body),
        ];
        let out = wrap_styled_line(&line, 12);
        // Whatever the row layout, each row should be exactly one
        // span (since every input span is the same style).
        for (i, row) in out.iter().enumerate() {
            assert_eq!(row.len(), 1, "row {i} should be one merged span");
        }
        assert!(out.len() >= 2, "should wrap into multiple rows");
    }
}
