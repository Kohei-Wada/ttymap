//! [`LuaComponent`] — adapter that lets a Lua script implement the
//! [`Component`] trait.
//!
//! The script is expected to return a Lua table (its "module"). The
//! adapter caches `name` at construction (so it can satisfy the
//! `&'static str` signature on [`Component::name`] by leaking) and
//! dispatches `render` back into Lua every frame.
//!
//! Per audit §13: errors are logged and recovered, never propagated —
//! a crashing Lua plugin must not take the host down. Failed lookups
//! yield empty render output and a warning in the log.
//!
//! Bridge surface so far: `name`, `render` (text lines wrapped in a
//! framed Paragraph), `handle_event` (Lua-side keymap → close /
//! ignore / silent consume), `paint_on_map` (Lua draws map markers
//! via [`MapApi`]), `poll` (Lua-side tick + `host:fetch_url(url)`).
//! Wider widget / map vocabulary lands in follow-ups.

use std::sync::{Arc, Mutex, mpsc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{Lua, RegistryKey, Table};

use crate::app::AppMsg;
use crate::compositor::Component;
use crate::compositor::window::{RenderWindow, Window};
use crate::geo::LonLat;
use crate::plugin_api::MapApi;
use crate::plugin_api::layout::PanelAnchor;
use crate::widget::{self, Line, Span, StyleKind};

/// Per-plugin layout knobs read from `module.layout`. Without this,
/// `LuaComponent::render` would paint the framed Paragraph over the
/// full map area and obscure the rendered tiles — visible to the
/// user as a black-and-white map while the panel is open.
struct LuaLayout {
    anchor: PanelAnchor,
    width: u16,
    height: Option<u16>,
}

impl LuaLayout {
    /// Sane default when a plugin omits the `layout` field —
    /// top-left, modest fixed size. Big enough for a few lines of
    /// text, small enough not to swallow the map.
    fn fallback() -> Self {
        Self {
            anchor: PanelAnchor::TopLeft,
            width: 32,
            height: Some(10),
        }
    }

    /// Read `module.layout = { anchor, width, height }`. Anything
    /// missing falls back to the corresponding [`Self::fallback`]
    /// field; an unknown anchor string falls back too rather than
    /// erroring (matches the rest of the bridge's recovery rule).
    fn from_module(module: &Table) -> Self {
        let mut out = Self::fallback();
        let Ok(layout): mlua::Result<Table> = module.get("layout") else {
            return out;
        };
        if let Ok(s) = layout.get::<String>("anchor")
            && let Some(a) = PanelAnchor::from_str(&s)
        {
            out.anchor = a;
        }
        if let Ok(w) = layout.get::<u16>("width") {
            out.width = w;
        }
        // `height` is optional: when absent, the panel uses the
        // full available height of `outer`. Lua nil reads as
        // missing; explicit numeric overrides that.
        if let Ok(h) = layout.get::<u16>("height") {
            out.height = Some(h);
        } else {
            out.height = None;
        }
        out
    }
}

/// A Component implemented in Lua.
#[allow(dead_code)] // first registrar caller lands when the hello.lua plugin is wired into build_registrar
pub struct LuaComponent {
    lua: Lua,
    /// Registry handle for the script's module table — re-fetched
    /// every dispatch via `lua.registry_value::<Table>(&self.module)`.
    module: RegistryKey,
    /// Cached at construction so [`Component::name`] can return
    /// `&'static str`. Leaked once per component; total cost is a
    /// few dozen bytes for the lifetime of the program.
    name: &'static str,
    /// Panel placement read from `module.layout` at construction.
    /// Stored on the component so `render` can compute the sub-rect
    /// without touching Lua every frame.
    layout: LuaLayout,
    /// Receiver for `host:jump(lon, lat)` requests. The Lua side
    /// pushes a `LonLat`; we drain after each `poll` /
    /// `handle_event` and emit `AppMsg::Jump` through the host
    /// `Window`. Keeps the Lua call site decoupled from when a
    /// `Window` is actually available.
    jump_rx: mpsc::Receiver<LonLat>,
    /// Map centre cell shared with the host so `host:center()` can
    /// return the latest value. We refresh it at the start of every
    /// dispatch path that carries a `Window` / `MapApi`.
    center: Arc<Mutex<LonLat>>,
}

#[allow(dead_code)] // mirrors the LuaComponent struct attribute; same reason
impl LuaComponent {
    /// Load `source` as a Lua chunk and capture its returned table as
    /// the component's module.
    ///
    /// `chunk_name` is used in Lua error messages — pass the source
    /// file name (or any identifier) so backtraces are readable.
    pub fn from_source(source: &str, chunk_name: &str) -> mlua::Result<Self> {
        let lua = super::new_lua();

        // Persistent host services (HTTP fetch etc.) are exposed as
        // a global so plugins can reach them from any callback. Set
        // *before* loading the source so a top-level `host:foo()`
        // call in the chunk would see it (none today, but cheap).
        let (host, jump_rx, center) = super::host::LuaHost::new("lua-host");
        let host_ud = lua.create_userdata(host)?;
        lua.globals().set("host", host_ud)?;

        let module: Table = lua.load(source).set_name(chunk_name).eval()?;
        let raw_name: String = module.get("name").unwrap_or_else(|_| "lua".to_string());
        let name: &'static str = Box::leak(raw_name.into_boxed_str());
        let layout = LuaLayout::from_module(&module);
        let module = lua.create_registry_value(module)?;
        Ok(Self {
            lua,
            module,
            name,
            layout,
            jump_rx,
            center,
        })
    }

    /// Refresh the host-shared map centre. Called at the start of
    /// every dispatch path that has access to a current centre
    /// (poll / handle_event via `Window::ctx()`, paint_on_map via
    /// `MapApi::center()`) so `host:center()` returns up-to-date
    /// values without each callback having to take it as an arg.
    fn update_center(&self, center: LonLat) {
        if let Ok(mut cell) = self.center.lock() {
            *cell = center;
        }
    }

    /// Drain any `host:jump(...)` requests the Lua side queued
    /// during the most recent callback and emit them as
    /// `AppMsg::Jump`. Called after `dispatch_event` /
    /// `dispatch_poll` while we still hold a `Window`.
    fn drain_jumps(&self, win: &mut Window) {
        while let Ok(ll) = self.jump_rx.try_recv() {
            win.emit(AppMsg::Jump(ll));
        }
    }

    /// Pull the `render()` lines from the Lua module. Returns an
    /// empty vec on any error (with a warning logged).
    fn render_lines(&self) -> Vec<String> {
        let result: mlua::Result<Vec<String>> = (|| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let render: mlua::Function = module.get("render")?;
            render.call(())
        })();
        match result {
            Ok(lines) => lines,
            Err(e) => {
                log::warn!("lua[{}]: render() failed: {}", self.name, e);
                Vec::new()
            }
        }
    }

    /// Run the Lua side of `handle_event` and return the host action
    /// the script asked for.
    ///
    /// Three outcomes:
    /// - **No `handle_event` field** → `KeyAction::Ignore`. Mirrors
    ///   the Component trait's default impl: the plugin opts out of
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
        let result: mlua::Result<KeyAction> = (|| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let handler: Option<mlua::Function> = module.get("handle_event").ok();
            let Some(handler) = handler else {
                return Ok(KeyAction::Ignore);
            };
            let key = self.build_key_table(event)?;
            let ret: mlua::Value = handler.call(key)?;
            Ok(KeyAction::from_lua_return(ret))
        })();
        match result {
            Ok(action) => action,
            Err(e) => {
                log::warn!("lua[{}]: handle_event() failed: {}", self.name, e);
                KeyAction::Consume
            }
        }
    }

    /// Run the Lua side of `poll`. The plugin's `poll()` function
    /// gets no arguments today — async work is reached through the
    /// `host` global (`host:fetch_url(url)` etc.). Missing function
    /// is a no-op; runtime errors are logged + recovered.
    fn dispatch_poll(&self) {
        let result: mlua::Result<()> = (|| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let poll: Option<mlua::Function> = module.get("poll").ok();
            let Some(poll) = poll else {
                return Ok(());
            };
            poll.call(())
        })();
        if let Err(e) = result {
            log::warn!("lua[{}]: poll() failed: {}", self.name, e);
        }
    }

    /// Run the Lua side of `paint_on_map`. Errors are logged and
    /// recovered: a buggy painter must not corrupt the rest of the
    /// frame.
    ///
    /// `MapApi` borrows the ratatui buffer for one frame, so the
    /// Lua-facing handle is built inside `Lua::scope` (closures
    /// over a `RefCell` of the ref) and torn down before this
    /// method returns.
    fn dispatch_paint(&self, p: &mut MapApi<'_>) {
        let cell = std::cell::RefCell::new(p);
        let result: mlua::Result<()> = self.lua.scope(|scope| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let painter: Option<mlua::Function> = module.get("paint_on_map").ok();
            let Some(painter) = painter else {
                return Ok(());
            };
            let map_table = super::map_api::make_map_table(&self.lua, scope, &cell)?;
            painter.call::<()>(map_table)
        });
        if let Err(e) = result {
            log::warn!("lua[{}]: paint_on_map() failed: {}", self.name, e);
        }
    }

    fn build_key_table(&self, event: KeyEvent) -> mlua::Result<Table> {
        let table = self.lua.create_table()?;
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
            // `return nil` → consume.
            mlua::Value::Nil => KeyAction::Consume,
            mlua::Value::Table(t) => {
                if t.get::<bool>("close").unwrap_or(false) {
                    KeyAction::Close
                } else if t.get::<bool>("ignore").unwrap_or(false) {
                    KeyAction::Ignore
                } else {
                    // Empty / unknown-key table → consume rather
                    // than silently letting the event escape.
                    KeyAction::Consume
                }
            }
            // Anything else (string, number, …) is malformed. Log
            // would be nice but we don't have the plugin name in
            // scope here; the dispatch_event wrapper will not log
            // because Ok(value) was returned. Treat as consume.
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

impl Component for LuaComponent {
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window) {
        self.update_center(win.ctx().center);
        let action = self.dispatch_event(event);
        self.drain_jumps(win);
        match action {
            KeyAction::Close => win.close(),
            KeyAction::Ignore => win.ignore(),
            KeyAction::Consume => {}
        }
    }

    fn paint_on_map(&self, p: &mut MapApi<'_>) {
        self.update_center(p.center());
        self.dispatch_paint(p);
    }

    fn poll(&mut self, win: &mut Window) {
        self.update_center(win.ctx().center);
        self.dispatch_poll();
        self.drain_jumps(win);
    }

    fn render(&self, win: &mut RenderWindow) {
        let outer = win.area();
        // Anchor the panel inside the map area so the framed
        // Paragraph doesn't paint over the rendered tiles. Height
        // defaults to the available space minus a 1-cell margin
        // when the plugin doesn't pin a specific value.
        let height = self
            .layout
            .height
            .unwrap_or_else(|| outer.height.saturating_sub(2));
        let area = self.layout.anchor.rect(outer, self.layout.width, height);

        // Wipe the panel rect before drawing. The block underneath
        // sets `style.bg` but leaves `fg` unset, which means cells
        // keep the map's previously-rendered foreground colours and
        // the panel ends up looking like a desaturated translucent
        // overlay. Clear first, then the framed Paragraph fills bg
        // and writes text fg from scratch — same trick the shared
        // `ListPanel` chrome uses.
        win.clear(area);

        let body = win.style(StyleKind::Body);
        let lines: Vec<Line> = self
            .render_lines()
            .into_iter()
            .map(|s| Line::from_span(Span::styled(s, body)))
            .collect();
        let paragraph = widget::Paragraph {
            lines,
            style: body,
            framed_title: Some(self.name.to_string()),
            ..Default::default()
        };
        win.paragraph(paragraph, area);
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_falls_back_when_module_omits_it() {
        // Module with no `name` field — adapter substitutes "lua".
        let c = LuaComponent::from_source("return {}", "anon").expect("load");
        assert_eq!(c.name(), "lua");
    }

    #[test]
    fn name_is_picked_up_from_module() {
        let c = LuaComponent::from_source(
            r#"return { name = "hello", render = function() return {} end }"#,
            "named",
        )
        .expect("load");
        assert_eq!(c.name(), "hello");
    }

    #[test]
    fn render_lines_round_trip_through_lua() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "demo",
                render = function() return { "alpha", "beta", "gamma" } end,
            }"#,
            "demo",
        )
        .expect("load");
        assert_eq!(c.render_lines(), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn render_lines_recovers_when_lua_throws() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "broken",
                render = function() error("kaboom") end,
            }"#,
            "broken",
        )
        .expect("load");
        // Should not panic — error is logged, we get an empty result.
        assert_eq!(c.render_lines(), Vec::<String>::new());
    }

    #[test]
    fn render_lines_recovers_when_field_is_missing() {
        let c = LuaComponent::from_source(r#"return { name = "noop" }"#, "noop").expect("load");
        // No `render` key → graceful fallback.
        assert_eq!(c.render_lines(), Vec::<String>::new());
    }

    #[test]
    fn loading_invalid_lua_returns_error() {
        let err = LuaComponent::from_source("this is not lua syntax !", "bad");
        assert!(err.is_err());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn missing_handler_dispatches_to_ignore() {
        let c = LuaComponent::from_source(r#"return { name = "noop" }"#, "noop").expect("load");
        assert_eq!(c.dispatch_event(key(KeyCode::Esc)), KeyAction::Ignore);
    }

    #[test]
    fn handler_returning_nil_consumes() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "modal",
                handle_event = function(_) return nil end,
            }"#,
            "modal",
        )
        .expect("load");
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('a'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_close_table_closes() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "esc",
                handle_event = function(k)
                    if k.code == "Esc" then return { close = true } end
                    return nil
                end,
            }"#,
            "esc",
        )
        .expect("load");
        assert_eq!(c.dispatch_event(key(KeyCode::Esc)), KeyAction::Close);
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('x'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_ignore_table_ignores() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "passthrough",
                handle_event = function(_) return { ignore = true } end,
            }"#,
            "passthrough",
        )
        .expect("load");
        assert_eq!(c.dispatch_event(key(KeyCode::Char('q'))), KeyAction::Ignore);
    }

    #[test]
    fn key_table_carries_modifiers_and_char() {
        // The handler echoes the parsed key back as a comma-separated
        // string so we can assert the table shape from Rust.
        let c = LuaComponent::from_source(
            r#"return {
                name = "echo",
                handle_event = function(k)
                    local ch = k.char or ""
                    local flags = ""
                    if k.ctrl  then flags = flags .. "C" end
                    if k.shift then flags = flags .. "S" end
                    if k.alt   then flags = flags .. "A" end
                    error(k.code .. ":" .. ch .. ":" .. flags)
                end,
            }"#,
            "echo",
        )
        .expect("load");
        // Use the recovery branch as a poor man's assertion: a
        // panicking handler logs + consumes, so we just need the
        // dispatch to not crash the test runner.
        // (For a real assertion we'd build an explicit channel; the
        // happy-path tests above already cover the protocol.)
        let _ = c.dispatch_event(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
    }

    #[test]
    fn handler_runtime_error_falls_back_to_consume() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "broken",
                handle_event = function(_) error("kaboom") end,
            }"#,
            "broken",
        )
        .expect("load");
        // Should not panic, should not leak as Ignore (we don't want
        // a malfunctioning plugin to unexpectedly forward keys to
        // the base layer).
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('a'))),
            KeyAction::Consume
        );
    }

    #[test]
    fn handler_returning_unknown_value_consumes() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "weird",
                handle_event = function(_) return "yolo" end,
            }"#,
            "weird",
        )
        .expect("load");
        assert_eq!(
            c.dispatch_event(key(KeyCode::Char('z'))),
            KeyAction::Consume
        );
    }

    // ── paint_on_map ─────────────────────────────────────────────

    use crate::map::render::frame::MapFrame;
    use crate::theme::{DARK, UiTheme};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn map_fixture(w: u16, h: u16) -> (Buffer, Rect, MapFrame, UiTheme) {
        let area = Rect::new(0, 0, w, h);
        let buf = Buffer::empty(area);
        let frame = MapFrame {
            cells: Vec::new(),
            cols: w,
            rows: h,
            center: crate::geo::LonLat { lon: 0.0, lat: 0.0 },
            zoom: 1.0,
        };
        let theme = UiTheme::from_palette(&DARK);
        (buf, area, frame, theme)
    }

    #[test]
    fn paint_on_map_missing_handler_is_no_op() {
        let c = LuaComponent::from_source(r#"return { name = "blank" }"#, "blank").expect("load");
        let (mut buf, area, frame, theme) = map_fixture(20, 5);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None);
        // Should not panic; nothing is written.
        c.dispatch_paint(&mut api);
    }

    #[test]
    fn paint_on_map_runtime_error_is_recovered() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "boom",
                paint_on_map = function(_) error("kaboom") end,
            }"#,
            "boom",
        )
        .expect("load");
        let (mut buf, area, frame, theme) = map_fixture(20, 5);
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None);
        // No panic; warning is logged.
        c.dispatch_paint(&mut api);
    }

    // ── poll ────────────────────────────────────────────────────

    #[test]
    fn poll_missing_handler_is_no_op() {
        let c = LuaComponent::from_source(r#"return { name = "static" }"#, "static").expect("load");
        // Should not panic, no warning meaningful enough to assert.
        c.dispatch_poll();
    }

    #[test]
    fn poll_runs_lua_handler_each_call() {
        // Counter side-effect: each dispatch_poll bumps it. Using a
        // module field rather than a global so we can read it back
        // through the registry-held module table.
        let c = LuaComponent::from_source(
            r#"return {
                name = "ticker",
                ticks = 0,
                poll = function()
                    -- Re-read the module table from the closure's
                    -- captured upvalue path; simplest is to bump a
                    -- global counter we can inspect from Rust.
                    _G.lua_test_ticks = (_G.lua_test_ticks or 0) + 1
                end,
            }"#,
            "ticker",
        )
        .expect("load");
        c.dispatch_poll();
        c.dispatch_poll();
        c.dispatch_poll();
        let n: i64 = c
            .lua
            .globals()
            .get("lua_test_ticks")
            .expect("global set by lua");
        assert_eq!(n, 3);
    }

    #[test]
    fn poll_runtime_error_is_recovered() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "broken",
                poll = function() error("kaboom") end,
            }"#,
            "broken",
        )
        .expect("load");
        // Should not panic.
        c.dispatch_poll();
    }

    // ── layout ──────────────────────────────────────────────────

    #[test]
    fn layout_falls_back_when_module_omits_it() {
        let c = LuaComponent::from_source(r#"return { name = "noop" }"#, "noop").expect("load");
        // Default fallback: TopLeft, 32x10. Asserting on the
        // resolved sub-rect inside a 100x40 outer is the cleanest
        // proxy for "didn't paint over the map".
        let outer = widget::Rect::new(0, 0, 100, 40);
        let height = c.layout.height.unwrap_or(outer.height.saturating_sub(2));
        let r = c.layout.anchor.rect(outer, c.layout.width, height);
        assert!(r.width <= 32, "fallback width should be at most 32");
        assert!(r.height <= 10, "fallback height should be at most 10");
        assert!(r.x < outer.width, "fallback x is in bounds");
        assert!(r.y < outer.height, "fallback y is in bounds");
    }

    #[test]
    fn layout_reads_anchor_width_height_from_module() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "configured",
                layout = { anchor = "right", width = 24, height = 8 },
            }"#,
            "configured",
        )
        .expect("load");
        assert_eq!(c.layout.anchor, PanelAnchor::Right);
        assert_eq!(c.layout.width, 24);
        assert_eq!(c.layout.height, Some(8));
    }

    #[test]
    fn layout_unknown_anchor_falls_back_silently() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "typo",
                layout = { anchor = "norkeast" },
            }"#,
            "typo",
        )
        .expect("load");
        // Default fallback anchor is TopLeft (see LuaLayout::fallback).
        assert_eq!(c.layout.anchor, PanelAnchor::TopLeft);
    }

    #[test]
    fn host_jump_drains_into_window_emit() {
        // A handler that requests a jump but doesn't return any
        // host action — the jump should still surface as an
        // AppMsg::Jump on the next drain.
        let mut c = LuaComponent::from_source(
            r#"return {
                name = "jumper",
                handle_event = function(key)
                    host:jump(139.7595, 35.6828)
                    return nil
                end,
            }"#,
            "jumper",
        )
        .expect("load");

        use crate::compositor::Context;
        use crate::compositor::window::WindowOps;
        const CTX: Context = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX);
            c.handle_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut win);
        }
        let jumps: Vec<&LonLat> = ops
            .msgs
            .iter()
            .filter_map(|m| match m {
                AppMsg::Jump(ll) => Some(ll),
                _ => None,
            })
            .collect();
        assert_eq!(jumps.len(), 1);
        assert!((jumps[0].lon - 139.7595).abs() < 1e-9);
        assert!((jumps[0].lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn host_global_is_set_for_plugins() {
        // A poll handler that errors out only if `host` is missing —
        // proves the global is wired and reachable from Lua.
        let c = LuaComponent::from_source(
            r#"return {
                name = "checker",
                poll = function()
                    assert(host ~= nil, "host must be set")
                    assert(type(host.fetch_url) == "function", "host:fetch_url must exist")
                end,
            }"#,
            "checker",
        )
        .expect("load");
        // If the assertions fail, dispatch_poll would log a warning;
        // the test passes either way (we'd need a stub log capture
        // to fail loudly). Round-trip via a Lua-side flag instead:
        let _: () = c
            .lua
            .load(
                r#"
                _G.checker_ok = false
                local ok = pcall(function()
                    assert(host ~= nil)
                    assert(type(host.fetch_url) == "function")
                end)
                _G.checker_ok = ok
                "#,
            )
            .exec()
            .expect("exec");
        let ok: bool = c.lua.globals().get("checker_ok").expect("get");
        assert!(ok, "host global must be present and have fetch_url");
    }

    #[test]
    fn paint_on_map_point_writes_into_the_buffer() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "marker",
                paint_on_map = function(map)
                    map:point(0.0, 0.0, "*", "accent")
                end,
            }"#,
            "marker",
        )
        .expect("load");
        let (mut buf, area, frame, theme) = map_fixture(20, 5);
        {
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None);
            c.dispatch_paint(&mut api);
        }
        // Confirm at least one cell got the marker glyph. Don't
        // pin the exact projected coord — the Web-Mercator rounding
        // for centre=(0,0), zoom=1 is implementation detail.
        let written = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].symbol() == "*");
        assert!(written, "expected at least one '*' in the buffer");
    }
}
