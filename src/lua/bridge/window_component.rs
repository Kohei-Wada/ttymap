//! [`LuaWindowComponent`] — a focused [`Component`] pushed onto the
//! compositor stack by `ttymap.api.window.open(spec)` (A3).
//!
//! Spec table fields (all optional unless layout dictates):
//! - `name = "..."` — display label shown in the focused-footer chip
//! - `layout = { anchor, width, height? }` — side-panel placement
//! - `render = function() return lines end` — panel body
//! - `handle_event = function(key) return action end` — focused keys
//! - `footer_hints = { {key, label}, ... }` — focused footer hints
//!
//! **No `paint_on_map`, no `poll`, no `loop`** — those belong on a
//! `ttymap.api.frame.on_tick(fn)` subscription (host-side).
//! A window opened via `window.open` does focused-UI work only; map
//! paint and async drain run in the per-frame tick on the main thread.
//!
//! Lifetime: the matching [`WindowHandle`] (returned to Lua by
//! `window.open`) carries a clone of the same [`CloseFlag`]. Either
//! side flipping the flag is honoured on the next [`Component::poll`]
//! tick, where this component pops itself off the stack via
//! [`Window::close`]. Idempotent — a flipped-then-flipped flag does
//! nothing extra.
//!
//! Drain plumbing (`ttymap.map:jump`, `ttymap.api.frame.export`)
//! lives in the **setup state** that ran the script's top-level
//! `register_*` calls — *not* on this per-window component. Those
//! receivers are returned by
//! [`crate::lua::api::install`] inside [`LuaHostHandles`] and
//! drained centrally by `App` per frame. This is by design:
//! `window.open` runs in the setup state's Lua VM, so its callbacks'
//! `ttymap.map:jump(...)` calls hit the setup-state senders, not
//! per-window receivers.
//!
//! Per audit §13: errors are logged and recovered, never propagated.
//! A buggy plugin must not take the host down.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{Lua, Table};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::handle::{CallOutcome, LuaHandle};
use super::window_handle::CloseFlag;
use crate::frontend::compositor::Component;
use crate::frontend::compositor::layout::PanelAnchor;
use crate::frontend::compositor::window::{RenderWindow, Window};
use crate::theme::StyleKind;

// ── Layout ─────────────────────────────────────────────────────────

/// Per-window layout knobs read from `spec.layout`.
#[derive(Debug, Clone)]
pub struct WindowLayout {
    anchor: PanelAnchor,
    width: u16,
    height: Option<u16>,
    placement: crate::frontend::compositor::Placement,
}

impl WindowLayout {
    /// Sane default when a plugin omits the `layout` field —
    /// top-left, 32×10, modal placement. Big enough for a few lines
    /// of text, small enough not to swallow the map.
    fn fallback() -> Self {
        Self {
            anchor: PanelAnchor::TopLeft,
            width: 32,
            height: Some(10),
            placement: crate::frontend::compositor::Placement::Modal,
        }
    }
}

/// Parse a `spec.layout.anchor` string into [`PanelAnchor`]. Unknown
/// strings yield `None` and the caller falls back to the layout
/// default.
fn parse_panel_anchor(s: &str) -> Option<PanelAnchor> {
    match s.to_ascii_lowercase().as_str() {
        "left" => Some(PanelAnchor::Left),
        "right" => Some(PanelAnchor::Right),
        "top-left" | "topleft" | "tl" => Some(PanelAnchor::TopLeft),
        "top-right" | "topright" | "tr" => Some(PanelAnchor::TopRight),
        "bottom-left" | "bottomleft" | "bl" => Some(PanelAnchor::BottomLeft),
        "bottom-right" | "bottomright" | "br" => Some(PanelAnchor::BottomRight),
        "center" | "centre" => Some(PanelAnchor::Center),
        _ => None,
    }
}

/// Read `spec.layout = { anchor, width, height }` with graceful
/// recovery: missing fields fall back to [`WindowLayout::fallback`],
/// unknown anchor strings fall back too rather than erroring (matches
/// the rest of the bridge's recovery rule).
fn parse_layout(spec: &Table) -> WindowLayout {
    let mut out = WindowLayout::fallback();
    let Ok(layout): mlua::Result<Table> = spec.get("layout") else {
        return out;
    };
    if let Ok(s) = layout.get::<String>("anchor")
        && let Some(a) = parse_panel_anchor(&s)
    {
        out.anchor = a;
    }
    if let Ok(w) = layout.get::<u16>("width") {
        out.width = w;
    }
    // `height` is optional: when absent the panel uses the full
    // available height of `outer`. Lua nil reads as missing; an
    // explicit numeric overrides that.
    if let Ok(h) = layout.get::<u16>("height") {
        out.height = Some(h);
    } else {
        out.height = None;
    }
    // `kind = "sidebar"` opts the component into the left sidebar's
    // vertical-section layout; absent or any other value keeps the
    // default modal placement.
    if let Ok(s) = layout.get::<String>("kind")
        && s.eq_ignore_ascii_case("sidebar")
    {
        out.placement = crate::frontend::compositor::Placement::Sidebar;
    }
    out
}

// ── Component ──────────────────────────────────────────────────────

/// A [`Component`] backed by a Lua spec table. Pushed onto the
/// compositor stack by `ttymap.api.window.open(spec)`; popped when
/// either side flips the shared [`CloseFlag`].
pub struct LuaWindowComponent {
    /// Bridge plumbing — fresh `Lua` VM, registered spec table,
    /// log tag (= identification used in warnings).
    handle: LuaHandle,
    /// Shared with the [`WindowHandle`](super::window_handle::WindowHandle)
    /// returned to Lua. Either side flipping it triggers a `win.close()`
    /// on the next poll tick.
    flag: CloseFlag,
    /// User-facing display label, read from `spec.name` if present
    /// at construction. Falls back to the handle's log tag (the
    /// `chunk_name` passed in by `window.open`). Leaked once so
    /// [`Component::name`] can satisfy the `&'static str` signature;
    /// bounded cost since `LuaWindowComponent` is rebuilt at most a
    /// few times per program lifetime.
    display: &'static str,
    /// Panel placement read from `spec.layout` at construction.
    layout: WindowLayout,
    /// Whether the spec exposes a `render` function. Marker-only
    /// windows (no panel UI) omit it; without this flag the adapter
    /// would still paint an empty framed Paragraph over the map.
    has_render: bool,
    /// Static footer hints from `spec.footer_hints`. Read once at
    /// construction so [`Component::footer_hints`] can hand back
    /// `&'static str` without leaking per call. Empty when the spec
    /// omits the field.
    footer_hints: Vec<(&'static str, &'static str)>,
}

impl LuaWindowComponent {
    /// Build a `LuaWindowComponent` from a spec table evaluated in
    /// `lua`. The spec is everything `window.open` was passed; the
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
    pub fn from_spec(
        lua: Lua,
        spec: Table,
        log_tag: &'static str,
        flag: CloseFlag,
    ) -> mlua::Result<Self> {
        // Display name: spec's `name` if set, else the log tag.
        // Leak once; bounded by the number of windows opened.
        let display: &'static str = spec
            .get::<String>("name")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
            .unwrap_or(log_tag);
        let layout = parse_layout(&spec);
        let has_render = matches!(
            spec.get::<mlua::Value>("render"),
            Ok(mlua::Value::Function(_))
        );
        let footer_hints = parse_footer_hints(&spec);
        let handle = LuaHandle::new(lua, spec, log_tag)?;
        Ok(Self {
            handle,
            flag,
            display,
            layout,
            has_render,
            footer_hints,
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

impl Component for LuaWindowComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        let action = self.dispatch_event(event);
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
        if !self.has_render {
            // Marker-only / map-only windows opt out of side-area
            // chrome; without this guard we'd paint an empty framed
            // Paragraph over the map.
            return;
        }
        let outer = win.area();
        // Anchor the panel inside the map area so the framed
        // Paragraph doesn't paint over the rendered tiles. Height
        // defaults to the available space minus a 1-cell margin
        // when the spec doesn't pin a specific value.
        let height = self
            .layout
            .height
            .unwrap_or_else(|| outer.height.saturating_sub(2));
        let area = self.layout.anchor.rect(outer, self.layout.width, height);
        let inner = win.panel(area, self.display);
        let body = win.style(StyleKind::Body);
        let lines: Vec<Line<'static>> = self
            .render_lines()
            .into_iter()
            .map(|spans| {
                let rendered: Vec<Span<'static>> = spans
                    .into_iter()
                    .map(|(text, kind)| Span::styled(text, win.style(kind)))
                    .collect();
                Line::from(rendered)
            })
            .collect();
        let paragraph = Paragraph::new(lines).style(body);
        win.paragraph(paragraph, inner);
    }

    fn poll(&mut self, win: &mut Window) {
        // The only Lua-facing poll work this Component does is
        // honour the shared close flag. NO callback into the spec —
        // async work belongs on a `ttymap.api.frame.on_tick(fn)`
        // subscription, not on the focused window.
        if self.flag.take() {
            win.close();
        }
    }

    fn name(&self) -> &'static str {
        self.display
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.footer_hints.clone()
    }

    fn placement(&self) -> crate::frontend::compositor::Placement {
        self.layout.placement
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Read `spec.footer_hints` as a sequence of `{key, label}` pairs and
/// leak each pair so [`Component::footer_hints`] can hand back
/// `&'static str`. Bounded leak — a window declares a finite list at
/// construction. Two accepted shapes per pair:
/// - `{ "Enter", "open" }` — positional 1-based array.
/// - `{ key = "Enter", label = "open" }` — named.
fn parse_footer_hints(spec: &Table) -> Vec<(&'static str, &'static str)> {
    let Ok(list): mlua::Result<Table> = spec.get("footer_hints") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in list.sequence_values::<mlua::Value>().flatten() {
        let mlua::Value::Table(pair) = entry else {
            continue;
        };
        let key: String = pair
            .get::<String>("key")
            .or_else(|_| pair.get::<String>(1))
            .unwrap_or_default();
        let label: String = pair
            .get::<String>("label")
            .or_else(|_| pair.get::<String>(2))
            .unwrap_or_default();
        if key.is_empty() && label.is_empty() {
            continue;
        }
        let key: &'static str = Box::leak(key.into_boxed_str());
        let label: &'static str = Box::leak(label.into_boxed_str());
        out.push((key, label));
    }
    out
}

/// Convert one Lua-returned line value into a vec of `(text, kind)`
/// spans. Bare string → single Body span. Table → array of
/// `{text, style}` records. Unknown style keywords fall back to
/// Body. Anything else stringifies to a single Body span so a buggy
/// plugin still renders.
fn parse_line_value(value: mlua::Value) -> Vec<(String, StyleKind)> {
    match value {
        mlua::Value::String(s) => {
            let text = s.to_str().map(|c| c.to_string()).unwrap_or_default();
            vec![(text, StyleKind::Body)]
        }
        mlua::Value::Table(t) => {
            let mut spans = Vec::new();
            for pair in t.sequence_values::<mlua::Value>().flatten() {
                if let mlua::Value::Table(span_t) = pair {
                    let text: String = span_t.get("text").unwrap_or_default();
                    let style: Option<String> = span_t.get("style").ok();
                    spans.push((text, style_from_str(style.as_deref())));
                } else if let mlua::Value::String(s) = pair {
                    let text = s.to_str().map(|c| c.to_string()).unwrap_or_default();
                    spans.push((text, StyleKind::Body));
                }
            }
            if spans.is_empty() {
                spans.push((String::new(), StyleKind::Body));
            }
            spans
        }
        other => vec![(format!("{:?}", other), StyleKind::Body)],
    }
}

/// Map a Lua-side style keyword to a [`StyleKind`]. Unknown values
/// fall back to `Body` so a typo paints in the default colour rather
/// than breaking the plugin.
fn style_from_str(name: Option<&str>) -> StyleKind {
    match name {
        Some("muted") => StyleKind::Muted,
        Some("accent") => StyleKind::Accent,
        Some("highlight") => StyleKind::Highlight,
        Some("selected") => StyleKind::Selected,
        Some("muted_fg") => StyleKind::MutedFg,
        Some("link") => StyleKind::Link,
        _ => StyleKind::Body,
    }
}

/// What the Lua `handle_event` handler asked the host to do.
#[derive(Debug, PartialEq, Eq)]
enum KeyAction {
    /// Pass the event to the base layer (Component default).
    Ignore,
    /// Pop the component off the stack.
    Close,
    /// Treat the event as handled with no further action.
    Consume,
}

impl KeyAction {
    fn from_lua_return(value: mlua::Value) -> Self {
        match value {
            mlua::Value::Nil => KeyAction::Consume,
            mlua::Value::Table(t) => {
                if t.get::<bool>("close").unwrap_or(false) {
                    KeyAction::Close
                } else if t.get::<bool>("ignore").unwrap_or(false) {
                    KeyAction::Ignore
                } else {
                    KeyAction::Consume
                }
            }
            _ => KeyAction::Consume,
        }
    }
}

/// Translate a crossterm `KeyCode` into the Lua-side `code` string
/// plus, for `Char(c)`, the actual character. Unknown variants
/// surface as `"Other"` so a Lua handler can at least see the event
/// arrived without reaching for the full crossterm vocabulary.
fn key_code_to_lua(code: KeyCode) -> (&'static str, Option<char>) {
    match code {
        KeyCode::Char(c) => ("Char", Some(c)),
        KeyCode::Enter => ("Enter", None),
        KeyCode::Esc => ("Esc", None),
        KeyCode::Tab => ("Tab", None),
        KeyCode::BackTab => ("BackTab", None),
        KeyCode::Backspace => ("Backspace", None),
        KeyCode::Up => ("Up", None),
        KeyCode::Down => ("Down", None),
        KeyCode::Left => ("Left", None),
        KeyCode::Right => ("Right", None),
        KeyCode::Home => ("Home", None),
        KeyCode::End => ("End", None),
        KeyCode::PageUp => ("PageUp", None),
        KeyCode::PageDown => ("PageDown", None),
        KeyCode::Delete => ("Delete", None),
        KeyCode::Insert => ("Insert", None),
        _ => ("Other", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal helper: build a `LuaWindowComponent` from a Lua source
    /// snippet that returns the spec table directly. `window.open`
    /// gets its spec the same way — caller-side `eval`, resulting
    /// Table handed in. Bypasses the whole `register_*` dance because
    /// these tests exercise component behaviour, not registration.
    fn make(source: &str, log_tag: &'static str) -> LuaWindowComponent {
        let lua = mlua::Lua::new();
        let spec: Table = lua.load(source).eval().expect("eval spec");
        LuaWindowComponent::from_spec(lua, spec, log_tag, CloseFlag::default()).expect("from_spec")
    }

    #[test]
    fn parse_layout_left_56() {
        let lua = mlua::Lua::new();
        let spec: Table = lua
            .load(r#"return { layout = { anchor = "left", width = 56 } }"#)
            .eval()
            .unwrap();
        let layout = parse_layout(&spec);
        assert_eq!(layout.anchor, PanelAnchor::Left);
        assert_eq!(layout.width, 56);
        assert_eq!(layout.height, None);
    }

    #[test]
    fn parse_layout_falls_back_when_missing() {
        let lua = mlua::Lua::new();
        let spec: Table = lua.load(r#"return {}"#).eval().unwrap();
        let layout = parse_layout(&spec);
        // Default fallback (matches WindowLayout::fallback).
        assert_eq!(layout.anchor, PanelAnchor::TopLeft);
        assert_eq!(layout.width, 32);
        assert_eq!(layout.height, Some(10));
    }

    #[test]
    fn parse_layout_unknown_anchor_falls_back_silently() {
        let lua = mlua::Lua::new();
        let spec: Table = lua
            .load(r#"return { layout = { anchor = "norkeast" } }"#)
            .eval()
            .unwrap();
        let layout = parse_layout(&spec);
        assert_eq!(layout.anchor, PanelAnchor::TopLeft);
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

    // ── close flag (the only poll-time work this Component does) ──

    #[test]
    fn poll_does_nothing_until_flag_is_flipped() {
        use crate::frontend::AppEvent;
        use crate::frontend::compositor::Context;
        use crate::frontend::compositor::window::WindowOps;

        let flag = CloseFlag::default();
        let lua = mlua::Lua::new();
        let spec: Table = lua.load(r#"return { name = "win" }"#).eval().unwrap();
        let mut c = LuaWindowComponent::from_spec(lua, spec, "win", flag.clone()).unwrap();

        const CTX: Context = Context {
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let (tx, _rx) = std::sync::mpsc::channel::<AppEvent>();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            c.poll(&mut win);
        }
        assert!(!ops.close, "no flag flip → no close queued");
    }

    #[test]
    fn poll_honours_flag_and_closes_window() {
        use crate::frontend::AppEvent;
        use crate::frontend::compositor::Context;
        use crate::frontend::compositor::window::WindowOps;

        let flag = CloseFlag::default();
        let lua = mlua::Lua::new();
        let spec: Table = lua.load(r#"return { name = "win" }"#).eval().unwrap();
        let mut c = LuaWindowComponent::from_spec(lua, spec, "win", flag.clone()).unwrap();

        const CTX: Context = Context {
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let (tx, _rx) = std::sync::mpsc::channel::<AppEvent>();
        flag.request();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            c.poll(&mut win);
        }
        assert!(ops.close, "flipped flag → win.close() queued");

        // Idempotent — second poll without re-flipping is a no-op.
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            c.poll(&mut win);
        }
        assert!(!ops.close);
    }
}
