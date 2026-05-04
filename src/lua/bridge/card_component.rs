//! [`LuaCardComponent`] — a focused [`Component`] pushed onto the
//! compositor stack by `ttymap.api.card.open(spec)` (A3).
//!
//! Spec table fields (all optional):
//! - `name = "..."` — display label shown in the focused-footer chip
//! - `render = function() return lines end` — panel body
//! - `handle_event = function(key) return action end` — focused keys
//! - `footer_hints = { {key, label}, ... }` — focused footer hints
//!
//! All `LuaCardComponent`s render in the left sidebar — there's no
//! free-floating / anchored layout for plugin-defined panels. The
//! `spec.layout` field is no longer read; existing scripts that set
//! it just see their setting silently ignored.
//!
//! **No `paint_on_map`, no `poll`, no `loop`** — those belong on a
//! `ttymap.api.frame.on_tick(fn)` subscription (host-side).
//! A window opened via `card.open` does focused-UI work only; map
//! paint and async drain run in the per-frame tick on the main thread.
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
//! lives in the **setup state** that ran the script's top-level
//! `register_*` calls — *not* on this per-window component. Those
//! receivers are returned by
//! [`crate::lua::api::install`] inside [`LuaHostHandles`] and
//! drained centrally by `App` per frame. This is by design:
//! `card.open` runs in the setup state's Lua VM, so its callbacks'
//! `ttymap.map:jump(...)` calls hit the setup-state senders, not
//! per-window receivers.
//!
//! Per audit §13: errors are logged and recovered, never propagated.
//! A buggy plugin must not take the host down.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{Lua, Table};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

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
/// the spec's `handle_event` returns `{ close = true }`.
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
    /// C-u in `handle_event` can use the *current* slot height as
    /// the page step size — `handle_event` itself has no Rect.
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
    /// The setup state owns the Sender / Receiver pairs for jump /
    /// frame.export and the host-shared `center` / `zoom` mutexes;
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

    /// Run the Lua side of `handle_event` and return the host action
    /// the script asked for.
    ///
    /// Three outcomes:
    /// - **No `handle_event` field** → `KeyAction::Ignore`. Mirrors
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
    fn dispatch_event(&self, event: KeyEvent) -> KeyAction {
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
        match self.handle.try_call::<_, mlua::Value>("handle_event", key) {
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

        let paragraph = Paragraph::new(lines).style(body).scroll((offset, 0));
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
        let list_items: Vec<ListItem<'static>> = raw_items
            .iter()
            .map(|item_lines| {
                let lines: Vec<Line<'static>> = item_lines
                    .iter()
                    .map(|spans| {
                        let rendered: Vec<Span<'static>> = spans
                            .iter()
                            .map(|(text, kind)| Span::styled(text.clone(), win.style(*kind)))
                            .collect();
                        Line::from(rendered)
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
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let action = self.dispatch_event(event);

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
        // setup state's senders, not per-window receivers. App drains
        // those centrally each frame.
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
        // Snapshot the slot's content height so handle_event can use
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

/// Read `spec.footer_hints` as a sequence of `{key, label}` pairs and
/// leak each pair so [`Component::footer_hints`] can hand back
/// `&'static str`. Bounded leak — a window declares a finite list at
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
        assert_eq!(c.dispatch_event(key(KeyCode::Esc)), KeyAction::Ignore);
    }

    #[test]
    fn handler_returning_nil_consumes() {
        let c = make(
            r#"return {
                name = "modal",
                handle_event = function(_) return nil end,
            }"#,
            "modal",
        );
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('a'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_close_table_closes() {
        let c = make(
            r#"return {
                name = "esc",
                handle_event = function(k)
                    if k.code == "Esc" then return { close = true } end
                    return nil
                end,
            }"#,
            "esc",
        );
        assert_eq!(c.dispatch_event(key(KeyCode::Esc)), KeyAction::Close);
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('x'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_ignore_table_ignores() {
        let c = make(
            r#"return {
                name = "passthrough",
                handle_event = function(_) return { ignore = true } end,
            }"#,
            "passthrough",
        );
        assert_eq!(c.dispatch_event(key(KeyCode::Char('q'))), KeyAction::Ignore);
    }

    #[test]
    fn handler_runtime_error_falls_back_to_consume() {
        let c = make(
            r#"return {
                name = "broken",
                handle_event = function(_) error("kaboom") end,
            }"#,
            "broken",
        );
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('a'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_unknown_value_consumes() {
        let c = make(
            r#"return {
                name = "weird",
                handle_event = function(_) return "yolo" end,
            }"#,
            "weird",
        );
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('z'))),
            KeyAction::Consume
        );
    }

    // Close coverage moved to `card_handle::tests::close_enqueues_op_close_idempotent`:
    // the close path no longer goes through Component::poll, so the
    // CloseFlag polling tests retire with the flag itself.
}
