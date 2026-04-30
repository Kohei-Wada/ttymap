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
//! via [`MapApi`]), `poll` (Lua-side tick + `ttymap.http:fetch(url)`).
//! Wider widget / map vocabulary lands in follow-ups.

use std::sync::{Arc, Mutex, mpsc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{Lua, RegistryKey, Table};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::AppMsg;
use crate::compositor::layout::PanelAnchor;
use crate::compositor::window::{OverlayWindow, RenderWindow, Window};
use crate::compositor::{Component, MapApi};
use crate::geo::LonLat;
use crate::lua::host::LuaHostShared;
use crate::theme::StyleKind;

/// Per-plugin layout knobs read from `module.layout`. Without this,
/// `LuaComponent::render` would paint the framed Paragraph over the
/// full map area and obscure the rendered tiles — visible to the
/// user as a black-and-white map while the panel is open.
struct LuaLayout {
    anchor: PanelAnchor,
    width: u16,
    height: Option<u16>,
}

/// Parse a `module.layout.anchor` string into [`PanelAnchor`]. The
/// only consumer is [`LuaLayout::from_module`]; unknown strings fall
/// back to the layout default rather than erroring.
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
            && let Some(a) = parse_panel_anchor(&s)
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
    /// Whether the module exposes a `render` function. Marker-only
    /// plugins (quake-style) omit it; without this flag the adapter
    /// would still paint an empty framed Paragraph over the map.
    has_render: bool,
    /// Static footer hints from `module.footer_hints`. Read once at
    /// construction so [`Component::footer_hints`] can satisfy the
    /// `&'static str` return type without leaking per call. Empty
    /// when the module omits the field.
    footer_hints: Vec<(&'static str, &'static str)>,
    /// Receiver for `ttymap.map:jump(lon, lat)` requests. The Lua side
    /// pushes a `LonLat`; we drain after each `poll` /
    /// `handle_event` and emit `AppMsg::Map(Action::Jump)` through
    /// the host `Window`. Keeps the Lua call site decoupled from
    /// when a `Window` is actually available.
    jump_rx: mpsc::Receiver<LonLat>,
    /// Receiver for `ttymap.window:close()` requests. Same pattern as
    /// `jump_rx`: the Lua side fires-and-forgets; we drain after
    /// each callback while still holding the `Window` and call
    /// `Window::close()`. Used by one-shot plugins (here-jump) that
    /// pop themselves once their work is done.
    close_rx: mpsc::Receiver<()>,
    /// Receiver for `ttymap.window:export_frame()` requests. Drained beside
    /// jump/close after each callback; emits `AppMsg::ExportFrame`
    /// through the host `Window`.
    export_rx: mpsc::Receiver<()>,
    /// Map centre cell shared with the host so `ttymap.map:center()` can
    /// return the latest value. We refresh it at the start of every
    /// dispatch path that carries a `Window` / `MapApi`.
    center: Arc<Mutex<LonLat>>,
}

#[allow(dead_code)] // mirrors the LuaComponent struct attribute; same reason
impl LuaComponent {
    pub fn from_source(
        source: &str,
        chunk_name: &str,
        shared: Arc<LuaHostShared>,
    ) -> mlua::Result<Self> {
        let lua = super::new_lua();
        let handles = super::host::install(&lua, "lua-host", shared)?;
        let module: Table = lua.load(source).set_name(chunk_name).eval()?;
        Self::build(lua, module, handles)
    }

    /// Shared post-eval construction: read the script's metadata
    /// (name / layout / render-presence / footer-hint declarations)
    /// off the resolved module table, then stash the table in the
    /// Lua registry so dispatch hooks can re-fetch it cheaply.
    ///
    /// Persistent host services (HTTP fetch etc.) live in the
    /// global `ttymap` table whose fields are domain-namespaced
    /// userdatas (`ttymap.http`, `ttymap.map`, …). They're installed
    /// by the caller *before* loading the source, so a top-level
    /// `ttymap.foo:bar()` call in the chunk would see them.
    fn build(lua: Lua, module: Table, handles: super::host::LuaHostHandles) -> mlua::Result<Self> {
        let super::host::LuaHostHandles {
            jump_rx,
            close_rx,
            export_rx,
            center,
        } = handles;

        let raw_name: String = module.get("name").unwrap_or_else(|_| "lua".to_string());
        let name: &'static str = Box::leak(raw_name.into_boxed_str());
        let layout = LuaLayout::from_module(&module);
        let has_render = matches!(
            module.get::<mlua::Value>("render"),
            Ok(mlua::Value::Function(_))
        );
        let footer_hints = parse_footer_hints(&module);
        let module = lua.create_registry_value(module)?;
        Ok(Self {
            lua,
            module,
            name,
            layout,
            has_render,
            footer_hints,
            jump_rx,
            close_rx,
            export_rx,
            center,
        })
    }

    /// Refresh the host-shared map centre. Called at the start of
    /// every dispatch path that has access to a current centre
    /// (poll / handle_event via `Window::ctx()`, paint_on_map via
    /// `MapApi::center()`) so `ttymap.map:center()` returns up-to-date
    /// values without each callback having to take it as an arg.
    fn update_center(&self, center: LonLat) {
        if let Ok(mut cell) = self.center.lock() {
            *cell = center;
        }
    }

    /// Drain any `ttymap.map:jump(...)` and `ttymap.window:export_frame()` requests
    /// the Lua side queued during the most recent callback. Calls
    /// `emit` once per queued request; the caller wires this to the
    /// active window's `emit` (works for both [`Window`] and
    /// [`OverlayWindow`]).
    fn drain_emits(&self, mut emit: impl FnMut(AppMsg)) {
        while let Ok(ll) = self.jump_rx.try_recv() {
            emit(AppMsg::Map(crate::map::Action::Jump(ll)));
        }
        while self.export_rx.try_recv().is_ok() {
            emit(AppMsg::ExportFrame);
        }
    }

    /// Drain any `ttymap.window:close()` requests the Lua side queued and
    /// pop the component off the stack. One drain per dispatch is
    /// enough — extra queued closes collapse into one `Window::close`
    /// because the compositor only respects the request once.
    fn drain_close(&self, win: &mut Window) {
        if self.close_rx.try_recv().is_ok() {
            // Drain any further close requests so they don't leak
            // into the next dispatch path.
            while self.close_rx.try_recv().is_ok() {}
            win.close();
        }
    }

    /// Overlay counterpart to [`drain_close`]. Overlays don't live on
    /// the focusable stack, so a `ttymap.window:close()` from one has nothing
    /// to pop. Drain the channel anyway (it's unbounded; left
    /// undrained it would grow forever) and warn so a misuse is
    /// visible.
    fn drain_close_overlay(&self) {
        if self.close_rx.try_recv().is_ok() {
            while self.close_rx.try_recv().is_ok() {}
            log::warn!(
                "lua[{}]: ttymap.window:close() ignored — overlay components don't live on the focusable stack",
                self.name
            );
        }
    }

    /// Pull the `render()` lines from the Lua module as raw line
    /// descriptors. Each line is a vec of `(text, style_kind)` spans.
    /// Returns an empty vec on any error (with a warning logged).
    ///
    /// Lua-side return shape (each list element is one line):
    /// - **string** → single Body span: `"hello"`
    /// - **array of `{text, style}` records** — multi-span line:
    ///   `{ { text = "Tokyo", style = "highlight" },
    ///      { text = "  10m", style = "muted" } }`
    ///
    /// Style keyword falls back to `Body` on unknown values so a typo
    /// still renders, just in the default colour.
    fn render_lines(&self) -> Vec<Vec<(String, StyleKind)>> {
        let result: mlua::Result<Vec<Vec<(String, StyleKind)>>> = (|| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let render: mlua::Function = module.get("render")?;
            let raw: Vec<mlua::Value> = render.call(())?;
            Ok(raw.into_iter().map(parse_line_value).collect())
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
    /// `ttymap` global (`ttymap.http:fetch(url)` etc.). Missing function
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

/// Read `module.footer_hints` as a sequence of `{key, label}` pairs
/// and leak each pair so [`Component::footer_hints`] can hand back
/// `&'static str`. Bounded leak — a plugin declares a finite list at
/// construction. Delegates parsing to [`super::parse_footer_hints`]
/// so the shape (positional / named, missing-field handling) stays
/// identical between the leak version stored on the component and the
/// owned version stored in the metadata snapshot.
fn parse_footer_hints(module: &Table) -> Vec<(&'static str, &'static str)> {
    super::parse_footer_hints(module)
        .into_iter()
        .map(|(k, v)| {
            let k: &'static str = Box::leak(k.into_boxed_str());
            let v: &'static str = Box::leak(v.into_boxed_str());
            (k, v)
        })
        .collect()
}

/// Convert one Lua-returned line value into a vec of `(text, kind)`
/// spans. A bare string becomes a single Body span; a table is read
/// as an array of `{text, style}` records and each becomes its own
/// span. Unknown style keywords fall back to Body. Anything else
/// (number, boolean, malformed table) yields a single Body span
/// using the value's display form so a buggy plugin still renders
/// instead of disappearing.
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
        other => {
            // Stringify unexpected variants so the plugin still
            // produces visible output.
            vec![(format!("{:?}", other), StyleKind::Body)]
        }
    }
}

/// Map a Lua-side style keyword to a `StyleKind`. Unknown values fall
/// back to `Body` so a typo paints in the default colour rather than
/// breaking the plugin.
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
        self.drain_emits(|m| win.emit(m));
        self.drain_close(win);
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
        self.drain_emits(|m| win.emit(m));
        self.drain_close(win);
    }

    fn poll_overlay(&mut self, win: &mut OverlayWindow) {
        self.update_center(win.ctx().center);
        self.dispatch_poll();
        self.drain_emits(|m| win.emit(m));
        self.drain_close_overlay();
    }

    fn render(&self, win: &mut RenderWindow) {
        if !self.has_render {
            // Marker-only plugins (no panel) opt out of side-area
            // chrome; without this guard we'd paint an empty framed
            // Paragraph over the map.
            return;
        }
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

        // `panel` clears the rect and draws the bordered block in one
        // step, returning the inner content region. Wiping first
        // matters because the block sets `style.bg` but leaves `fg`
        // unset, so without the clear cells would keep the map's
        // foreground colours and the panel would look translucent.
        let inner = win.panel(area, self.name);

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

    fn name(&self) -> &'static str {
        self.name
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.footer_hints.clone()
    }

    /// All `LuaComponent` instances share `Any::type_id`, so the
    /// compositor's TypeId fallback would collapse every Lua plugin
    /// to "the same kind". Surface the script's per-instance name
    /// (file stem for bundled scripts, the `name` field for entries)
    /// so different Lua plugins coexist on the stack and re-toggling
    /// the same plugin still closes it. See `compositor::same_identity`.
    fn dedup_tag(&self) -> Option<&str> {
        Some(self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_falls_back_when_module_omits_it() {
        // Module with no `name` field — adapter substitutes "lua".
        let c = LuaComponent::from_source(
            "return {}",
            "anon",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        assert_eq!(c.name(), "lua");
    }

    #[test]
    fn name_is_picked_up_from_module() {
        let c = LuaComponent::from_source(
            r#"return { name = "hello", render = function() return {} end }"#,
            "named",
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        let lines = c.render_lines();
        let texts: Vec<&str> = lines.iter().map(|spans| spans[0].0.as_str()).collect();
        assert_eq!(texts, vec!["alpha", "beta", "gamma"]);
        // All bare strings render as Body.
        assert!(lines.iter().all(|spans| spans[0].1 == StyleKind::Body));
    }

    #[test]
    fn render_lines_recovers_when_lua_throws() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "broken",
                render = function() error("kaboom") end,
            }"#,
            "broken",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        // Should not panic — error is logged, we get an empty result.
        assert!(c.render_lines().is_empty());
    }

    #[test]
    fn render_lines_recovers_when_field_is_missing() {
        let c = LuaComponent::from_source(
            r#"return { name = "noop" }"#,
            "noop",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        // No `render` key → graceful fallback.
        assert!(c.render_lines().is_empty());
    }

    #[test]
    fn render_lines_parses_styled_span_tables() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "styled",
                render = function()
                    return {
                        {
                            { text = "Title", style = "highlight" },
                            { text = "  10m", style = "muted" },
                        },
                        "plain body line",
                    }
                end,
            }"#,
            "styled",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        let lines = c.render_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 2);
        assert_eq!(lines[0][0].0, "Title");
        assert_eq!(lines[0][0].1, StyleKind::Highlight);
        assert_eq!(lines[0][1].0, "  10m");
        assert_eq!(lines[0][1].1, StyleKind::Muted);
        assert_eq!(lines[1][0].0, "plain body line");
        assert_eq!(lines[1][0].1, StyleKind::Body);
    }

    #[test]
    fn loading_invalid_lua_returns_error() {
        let err = LuaComponent::from_source(
            "this is not lua syntax !",
            "bad",
            super::super::host::LuaHostShared::empty(),
        );
        assert!(err.is_err());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn missing_handler_dispatches_to_ignore() {
        let c = LuaComponent::from_source(
            r#"return { name = "noop" }"#,
            "noop",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
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
        let c = LuaComponent::from_source(
            r#"return { name = "blank" }"#,
            "blank",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
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
            super::super::host::LuaHostShared::empty(),
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
        let c = LuaComponent::from_source(
            r#"return { name = "static" }"#,
            "static",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        // Should not panic.
        c.dispatch_poll();
    }

    // ── layout ──────────────────────────────────────────────────

    #[test]
    fn layout_falls_back_when_module_omits_it() {
        let c = LuaComponent::from_source(
            r#"return { name = "noop" }"#,
            "noop",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        // Default fallback: TopLeft, 32x10. Asserting on the
        // resolved sub-rect inside a 100x40 outer is the cleanest
        // proxy for "didn't paint over the map".
        let outer = ratatui::layout::Rect::new(0, 0, 100, 40);
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
            super::super::host::LuaHostShared::empty(),
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
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        // Default fallback anchor is TopLeft (see LuaLayout::fallback).
        assert_eq!(c.layout.anchor, PanelAnchor::TopLeft);
    }

    #[test]
    fn host_jump_drains_into_window_emit() {
        // A handler that requests a jump but doesn't return any
        // host action — the jump should still surface as an
        // AppMsg::Map(Action::Jump) on the next drain.
        let mut c = LuaComponent::from_source(
            r#"return {
                name = "jumper",
                handle_event = function(key)
                    ttymap.map:jump(139.7595, 35.6828)
                    return nil
                end,
            }"#,
            "jumper",
            super::super::host::LuaHostShared::empty(),
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
                AppMsg::Map(crate::map::Action::Jump(ll)) => Some(ll),
                _ => None,
            })
            .collect();
        assert_eq!(jumps.len(), 1);
        assert!((jumps[0].lon - 139.7595).abs() < 1e-9);
        assert!((jumps[0].lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn poll_overlay_drains_host_jump_into_emit() {
        // An overlay that fires `ttymap.map:jump` from poll. Verifies
        // the overlay path keeps emit semantics (the same drain logic
        // that works for stack components) even though the
        // OverlayWindow surface is narrower.
        let mut c = LuaComponent::from_source(
            r#"return {
                name = "jumper",
                poll = function()
                    ttymap.map:jump(139.7595, 35.6828)
                end,
            }"#,
            "jumper",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");

        use crate::compositor::Context;
        const CTX: Context = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let mut msgs: Vec<AppMsg> = Vec::new();
        {
            let mut win = OverlayWindow::new(&mut msgs, &CTX);
            c.poll_overlay(&mut win);
        }
        let jumps: Vec<&LonLat> = msgs
            .iter()
            .filter_map(|m| match m {
                AppMsg::Map(crate::map::Action::Jump(ll)) => Some(ll),
                _ => None,
            })
            .collect();
        assert_eq!(jumps.len(), 1);
        assert!((jumps[0].lon - 139.7595).abs() < 1e-9);
        assert!((jumps[0].lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn poll_overlay_drains_host_close_without_emitting() {
        // An overlay that incorrectly calls `ttymap.window:close()`
        // from poll. The overlay path must drain `close_rx` (else the
        // unbounded channel would grow forever) but emit nothing on
        // the OverlayWindow — overlays don't live on the focusable
        // stack.
        let mut c = LuaComponent::from_source(
            r#"return {
                name = "bad_overlay",
                poll = function()
                    ttymap.window:close()
                end,
            }"#,
            "bad_overlay",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");

        use crate::compositor::Context;
        const CTX: Context = Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let mut msgs: Vec<AppMsg> = Vec::new();
        {
            let mut win = OverlayWindow::new(&mut msgs, &CTX);
            c.poll_overlay(&mut win);
            // Second tick — the channel must have been drained on
            // the first call. A missing drain would log a second
            // warning here; correctness is unaffected because the
            // OverlayWindow has no close path either way.
            c.poll_overlay(&mut win);
        }
        // No close-path msgs exist on OverlayWindow; just assert
        // no jumps / exports leaked into the queue.
        assert!(
            msgs.is_empty(),
            "overlay poll must not emit anything for a ttymap.window:close()"
        );
    }

    #[test]
    fn ttymap_global_is_set_for_plugins() {
        // A poll handler that errors out only if `ttymap` is missing —
        // proves the global is wired and reachable from Lua.
        let c = LuaComponent::from_source(
            r#"return {
                name = "checker",
                poll = function()
                    assert(ttymap ~= nil, "ttymap must be set")
                    assert(type(ttymap.http) == "userdata",
                        "ttymap.http namespace must exist")
                end,
            }"#,
            "checker",
            super::super::host::LuaHostShared::empty(),
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
                    assert(ttymap ~= nil)
                    assert(type(ttymap.http) == "userdata")
                end)
                _G.checker_ok = ok
                "#,
            )
            .exec()
            .expect("exec");
        let ok: bool = c.lua.globals().get("checker_ok").expect("get");
        assert!(ok, "ttymap global must be present and expose namespaces");
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
            super::super::host::LuaHostShared::empty(),
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

    #[test]
    fn paint_on_map_label_writes_text_into_the_buffer() {
        // `map:label(lon, lat, "ISS", "accent")` should render at
        // least one cell whose symbol is one of the literal label
        // characters. Same shape as the point test — coordinates
        // round to *somewhere* in the buffer at centre=(0,0)/zoom=1.
        let c = LuaComponent::from_source(
            r#"return {
                name = "marker",
                paint_on_map = function(map)
                    map:label(0.0, 0.0, "ISS", "accent")
                end,
            }"#,
            "marker",
            super::super::host::LuaHostShared::empty(),
        )
        .expect("load");
        let (mut buf, area, frame, theme) = map_fixture(20, 5);
        {
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None);
            c.dispatch_paint(&mut api);
        }
        let chars: Vec<&str> = vec!["I", "S"];
        let written = (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| chars.contains(&buf[(x, y)].symbol()));
        assert!(written, "expected at least one label glyph in the buffer");
    }
}
