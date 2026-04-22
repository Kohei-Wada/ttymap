//! Compositor prototype — helix-style stack-based modal system.
//!
//! **This is a design prototype.** It is compiled (to verify the
//! types are coherent) but not wired into `App`. The goal is to
//! replace the current `FocusManager` + `FocusSurface` + `Plugin`
//! trilogy with a simpler compositor stack, as sketched in the
//! Window API discussion.
//!
//! Inspired by `helix-view/src/compositor.rs` (Helix). Ttymap's fit
//! for this pattern: the UX is "always-on map + modal popups
//! (search, palette, help, wiki)", which is exactly what a
//! compositor stack models — push on open, pop on close, focus =
//! stack top.
//!
//! # What goes away under this model
//!
//! | Today (Plugin + FocusSurface)                     | Compositor               |
//! |---------------------------------------------------|--------------------------|
//! | `fn is_visible(&self) -> bool` (per plugin)       | existence on stack = visible |
//! | `fn activate(&mut self, ctx)`                     | `push(Box::new(T::new(ctx)))` — no separate hook |
//! | `fn deactivate(&mut self)`                        | `Drop` — automatic on pop |
//! | `Effect::{Pass, Consumed, Run, Open}`             | `EventResult::{Ignored, Consumed, Close}` |
//! | `activation_keys() + BackgroundResponder` lookup  | bottom-layer component pushes the modal |
//! | `FocusManager::cycle` + `Focus::{Background, Modal}` state machine | `Compositor::stack` and top-is-focused |
//! | `FocusManager::release_focused` auto-release      | `Close` pops, no state to flip |
//!
//! A prototype `SearchComponent` at the bottom of this file shows
//! what one plugin looks like after conversion.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::AppMsg;
use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::painter::MapPainter;
use crate::theme::UiTheme;

/// Outcome of delivering an event to a [`Component`]. Helix uses the
/// same three-variant shape; the semantics are:
///
/// - `Ignored`: the component is not interested. Compositor tries
///   the next layer down. If nothing claims it, the event is
///   discarded (or — in the real wiring — the keymap gets it via
///   the bottom "always-there" layer).
/// - `Consumed(msgs)`: the component absorbed the event and
///   optionally emits messages. Propagation stops here.
/// - `Close(msgs)`: consumed + the component asks to be popped.
///   Messages are dispatched first so e.g. a `Jump` fires before
///   the component disappears.
pub enum EventResult {
    Ignored,
    Consumed(Vec<AppMsg>),
    Close(Vec<AppMsg>),
}

/// Read-only snapshot of app-level context a component might need
/// during key handling. Equivalent to today's `SurfaceCtx`.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub center: LonLat,
    pub theme_id: ThemeId,
}

/// A focus-capable UI entity.
///
/// **Lifecycle = object lifetime.** A component is `push`ed when the
/// user activates the corresponding surface; it is `drop`ped when it
/// returns `Close`. There is no separate `activate` / `deactivate` /
/// `is_visible` contract to forget to update — existence on the
/// stack *is* visibility, and `Close` *is* the deactivation.
pub trait Component {
    /// Handle a single key event. Return `Ignored` to let lower
    /// layers see it, `Consumed(msgs)` to absorb, or `Close(msgs)`
    /// to absorb and pop.
    fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> EventResult;

    /// Paint this component into `area`. Called once per frame while
    /// on the stack. The compositor renders bottom-to-top so later
    /// pushes draw on top.
    fn render(&self, f: &mut Frame, area: Rect, theme: &UiTheme);

    /// Advance any async work and surface new messages. Called every
    /// tick on every component on the stack. Replaces the old
    /// `Plugin::poll() + Plugin::pending_msgs()` pair — one hook
    /// instead of two.
    fn poll(&mut self) -> Vec<AppMsg> {
        Vec::new()
    }

    /// Footer hints shown while this component is on top of the
    /// stack. (In the final wiring, `Compositor::footer_hints()` reads
    /// the top; lower layers are invisible to the footer.)
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}

/// Stack of modal components. Top of stack holds focus.
///
/// This is the **replacement for `FocusManager`**. The state machine
/// (`Focus::Background` / `Focus::Modal(SurfaceId)`, `prev` memory,
/// `release_focused`, `transition_to`) all collapse into "push/pop a
/// `Vec`".
pub struct Compositor {
    stack: Vec<Box<dyn Component>>,
}

impl Compositor {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// Push a new component on top. It immediately holds focus.
    pub fn push(&mut self, c: Box<dyn Component>) {
        self.stack.push(c);
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Deliver a key event top-to-bottom until something takes it.
    /// Returns the messages the handling component emitted (empty if
    /// nothing claimed the event — the caller may then run its own
    /// fallback, e.g. the keymap).
    pub fn handle_event(&mut self, event: KeyEvent, ctx: &Context) -> Vec<AppMsg> {
        for i in (0..self.stack.len()).rev() {
            match self.stack[i].handle_event(event, ctx) {
                EventResult::Ignored => continue,
                EventResult::Consumed(msgs) => return msgs,
                EventResult::Close(msgs) => {
                    self.stack.remove(i);
                    return msgs;
                }
            }
        }
        Vec::new()
    }

    /// Poll every component. Each component's messages are appended
    /// in stack order (bottom first).
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

    /// Footer hints from the top of the stack, or empty if no modal
    /// is up (caller falls back to the keymap's default hints).
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

// ── Plugin self-registration (App is plugin-agnostic) ──────────────
//
// **Design rule**: `App` must not know any concrete plugin type.
// Today's `build_plugin_registry` in `app::mod` names every plugin
// (`SearchPlugin`, `WikiPlugin`, …) and its constructor — that's
// tech debt we inherit and should not expand. Going forward, only
// the composition root (main.rs, or a dedicated `plugins.rs`) names
// plugins; `App` takes a finished [`Registrar`] and doesn't care
// which plugins produced which entries.
//
// Each plugin module exposes a free function
//
//     pub fn register(config: &Config, r: &mut Registrar)
//
// that constructs its own state (e.g. `Rc<RefCell<WikiState>>`),
// captures it into closures, and calls the `Registrar` methods below.
// App's composition root just iterates every plugin's `register`;
// it never sees `WikiState`, `SearchService`, or any plugin struct.
//
// 3rd-party plugins become trivial: add `pub fn register(...)` in a
// sibling crate, call it from main, done.

/// Factory closure that produces a fresh [`Component`] when the user
/// activates the corresponding surface. Captures whatever handles
/// the plugin needs (e.g. a shared state handle, async service).
pub type SpawnComponent = Box<dyn Fn() -> Box<dyn Component>>;

/// One activation entry — "when this key is pressed while nothing is
/// on the stack, invoke `spawn` and push the result". Keymap details
/// (parsing, modifiers) live in [`crate::keymap`]; this uses
/// `KeyEvent` directly.
pub struct Activation {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub spawn: SpawnComponent,
}

/// Palette entry description owned by the registrar. Real conversion
/// would reuse / replace the existing `PaletteItem`; kept opaque
/// here because the prototype only cares about wiring shape.
pub struct PaletteEntry {
    pub label: String,
    pub spawn: SpawnComponent,
}

/// Collector passed to each plugin's `register` function. All four
/// channels (painter / activation / palette / async task) are
/// optional — headless plugins (`here`) add nothing but a palette
/// entry + async task; visual plugins (`search`) add an activation
/// + palette entry; `wiki` adds all four.
#[derive(Default)]
pub struct Registrar {
    pub painters: Vec<Box<dyn Painter>>,
    pub activations: Vec<Activation>,
    pub palette_entries: Vec<PaletteEntry>,
    // Async-only plugins (the `here` case) will need one more channel
    // — probably `tasks: Vec<Box<dyn Task>>` where `Task::poll() ->
    // Vec<AppMsg>`. Omitted from this prototype until `here`'s shape
    // is pinned down (see Known design gaps below).
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
}

// ── Map painters (always-on, focus-independent) ────────────────────
//
// **Why a separate trait, not a `Component` method.**
//
// Some plugins draw world-space primitives on the map *regardless of
// focus* — today that's `wiki` dropping a '●' on every article even
// when the panel is closed. That's incompatible with the compositor
// rule "on the stack = visible", because the wiki panel may be off
// the stack while its markers are still wanted.
//
// The cleanest resolution is to **split the two concerns**:
//
// - [`Component`] = modal UI + key handling, ephemeral, lives on the
//   stack while focused
// - [`Painter`] = world-space overlay, permanent, lives in an
//   app-level list and is drawn every frame
//
// Plugins with both (wiki) implement both traits and share state
// through `Rc<RefCell<State>>` so the two views of the same logical
// plugin stay in sync. See `WikiState` / `WikiComponent` /
// `WikiPainter` below.
pub trait Painter {
    fn paint(&self, p: &mut MapPainter<'_>);
}

// ── Prototype: search as a Component ────────────────────────────────
//
// This is the equivalent of today's `src/plugin/search/mod.rs` under
// the compositor model. The whole `impl Plugin` + `impl FocusSurface`
// block collapses into a single `impl Component`. Note what is
// *gone*: `tag`, `description`, `activation_keys`, `wants_focus`,
// `activate`, `deactivate`, `is_visible`, `pending_msgs`. None of
// them have an analogue because the compositor handles them
// implicitly.

/// Minimal stand-in for `SearchService` — the prototype doesn't need
/// a real HTTP backend; it just has to match the existing shape so
/// the `handle_event` code below is a fair line-for-line comparison
/// with `plugin::search::mod::handle_key`.
#[derive(Default)]
struct StubSearchService {
    polled_once: bool,
}

#[derive(Clone, Debug)]
struct StubResult {
    pub location: LonLat,
}

impl StubSearchService {
    fn search(&mut self, _q: &str) {}
    fn poll(&mut self) -> Option<Vec<StubResult>> {
        None
    }
}

pub struct SearchComponent {
    query: String,
    candidates: Vec<StubResult>,
    selected: usize,
    service: StubSearchService,
}

impl SearchComponent {
    /// Created fresh on every push. Replaces `SearchPlugin::open` —
    /// no need to reset state because this *is* a brand new instance.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            candidates: Vec::new(),
            selected: 0,
            service: StubSearchService::default(),
        }
    }

    fn has_candidates(&self) -> bool {
        !self.candidates.is_empty()
    }
}

impl Component for SearchComponent {
    fn handle_event(&mut self, event: KeyEvent, _ctx: &Context) -> EventResult {
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);

        if self.has_candidates() {
            let up = matches!(event.code, KeyCode::Up | KeyCode::Char('k'))
                || (ctrl && event.code == KeyCode::Char('p'));
            let down = matches!(event.code, KeyCode::Down | KeyCode::Char('j'))
                || (ctrl && event.code == KeyCode::Char('n'));

            return match event.code {
                KeyCode::Esc => EventResult::Close(Vec::new()),
                KeyCode::Enter => {
                    let loc = self.candidates[self.selected].location;
                    EventResult::Close(vec![AppMsg::Jump(loc)])
                }
                _ if up => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                    EventResult::Consumed(Vec::new())
                }
                _ if down => {
                    if self.selected + 1 < self.candidates.len() {
                        self.selected += 1;
                    }
                    EventResult::Consumed(Vec::new())
                }
                _ => EventResult::Consumed(Vec::new()),
            };
        }

        match event.code {
            KeyCode::Esc => EventResult::Close(Vec::new()),
            KeyCode::Enter => {
                if self.query.is_empty() {
                    EventResult::Close(Vec::new())
                } else {
                    self.service.search(&self.query);
                    EventResult::Consumed(Vec::new())
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                EventResult::Consumed(Vec::new())
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                EventResult::Consumed(Vec::new())
            }
            _ => EventResult::Consumed(Vec::new()),
        }
    }

    fn render(&self, _f: &mut Frame, _area: Rect, _theme: &UiTheme) {
        // Delegates to panel::render_panel in real conversion —
        // stubbed here because panel takes a concrete &SearchPlugin.
    }

    fn poll(&mut self) -> Vec<AppMsg> {
        // Drains completed search results. If we had them, we'd push
        // them into self.candidates here; no separate pending_msgs()
        // hop.
        if let Some(results) = self.service.poll() {
            self.candidates = results;
            self.selected = 0;
        }
        Vec::new()
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.has_candidates() {
            vec![("↑↓", "select"), ("Enter", "jump"), ("Esc", "cancel")]
        } else {
            vec![("Enter", "search"), ("Esc", "cancel"), ("C-u", "clear")]
        }
    }
}

impl Default for SearchComponent {
    fn default() -> Self {
        Self::new()
    }
}

// ── Prototype: wiki as Component + Painter with persistent state ───
//
// Wiki is the hardest fit for a pure "object lifetime = visibility"
// compositor because:
//
// 1. The marker list persists across panel open/close (user pans the
//    map, opens wiki, closes panel → markers stay on the map).
// 2. The panel, when reopened, remembers the previous selection /
//    scroll position.
// 3. Map markers are drawn regardless of whether the panel holds
//    focus.
//
// The resolution splits the concerns:
//
// - `WikiState` = the shared, persistent state (articles, selection,
//   service, throttle). Owned by `App` via an `Rc<RefCell<_>>`
//   handle. Lives for the whole app lifetime.
// - `WikiComponent` = the focus-side view. Pushed onto the compositor
//   when the user activates the panel; popped on Esc. Holds a clone
//   of the `Rc<RefCell<WikiState>>`, reading and mutating the shared
//   state through it. Cheap to construct on each push because the
//   state is behind the Rc.
// - `WikiPainter` = the map-side view. Registered in `App`'s painter
//   list once at startup. Holds a clone of the same
//   `Rc<RefCell<WikiState>>` and draws a '●' at each article location
//   every frame, regardless of whether the component is on the stack.
//
// This is the same split Helix uses for editor overlays — a handle
// to shared state threaded through multiple surface views.

/// Stub stand-in for `wikipedia::WikiArticle` — only the fields the
/// prototype touches are here.
#[derive(Clone)]
struct StubWikiArticle {
    title: String,
    lon: f64,
    lat: f64,
}

/// Shared wiki state. The real conversion would include
/// `service: WikiService`, `throttle: Throttle`, etc.; both are
/// omitted here because they don't affect the focus/painter shape.
#[derive(Default)]
pub struct WikiState {
    articles: Vec<StubWikiArticle>,
    selected: usize,
    detail: Option<StubWikiArticle>,
}

impl WikiState {
    fn is_detail_open(&self) -> bool {
        self.detail.is_some()
    }
}

/// Handle to shared wiki state. `App` builds one and hands clones to
/// `WikiComponent` (on push) and `WikiPainter` (registered at
/// startup). `Rc<RefCell<_>>` is fine because all three — App,
/// compositor, painter list — live on the main thread.
pub type WikiHandle = Rc<RefCell<WikiState>>;

/// Focus-side view of wiki. Pushed on activation; popped on Esc.
/// **No per-view state** — every bit of state is behind the handle,
/// so push/pop is genuinely cheap and cannot desync from the painter.
pub struct WikiComponent {
    state: WikiHandle,
}

impl WikiComponent {
    pub fn new(state: WikiHandle) -> Self {
        Self { state }
    }
}

impl Component for WikiComponent {
    fn handle_event(&mut self, event: KeyEvent, _ctx: &Context) -> EventResult {
        let mut state = self.state.borrow_mut();
        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        let up = (ctrl && event.code == KeyCode::Char('p')) || event.code == KeyCode::Up;
        let down = (ctrl && event.code == KeyCode::Char('n')) || event.code == KeyCode::Down;
        let exit_detail = matches!(
            event.code,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter
        );

        // `i` (the activation key) closes: no internal flag to flip,
        // just pop. Real keymap integration happens at the
        // bottom-layer component — here we just handle the modal's
        // own keys.
        if event.code == KeyCode::Char('i') && event.modifiers == KeyModifiers::NONE {
            return EventResult::Close(Vec::new());
        }

        if state.articles.is_empty() {
            return if up || down || exit_detail {
                EventResult::Consumed(Vec::new())
            } else {
                EventResult::Ignored // empty panel doesn't claim foreign keys
            };
        }

        // Detail mode
        if state.is_detail_open() {
            if exit_detail {
                state.detail = None;
                return EventResult::Consumed(Vec::new());
            }
            if up || down {
                let n = state.articles.len();
                state.selected = if up {
                    if state.selected == 0 { n - 1 } else { state.selected - 1 }
                } else {
                    (state.selected + 1) % n
                };
                let article = state.articles[state.selected].clone();
                let loc = LonLat { lat: article.lat, lon: article.lon };
                state.detail = Some(article);
                return EventResult::Consumed(vec![AppMsg::Jump(loc)]);
            }
            return EventResult::Consumed(Vec::new());
        }

        // List mode
        if event.code == KeyCode::Enter {
            let article = state.articles[state.selected].clone();
            let loc = LonLat { lat: article.lat, lon: article.lon };
            state.detail = Some(article);
            return EventResult::Consumed(vec![AppMsg::Jump(loc)]);
        }
        if up || down {
            let n = state.articles.len();
            state.selected = if up {
                if state.selected == 0 { n - 1 } else { state.selected - 1 }
            } else {
                (state.selected + 1) % n
            };
            let article = &state.articles[state.selected];
            return EventResult::Consumed(vec![AppMsg::Jump(LonLat {
                lat: article.lat,
                lon: article.lon,
            })]);
        }

        EventResult::Ignored // non-modal: let lower layers handle unknown keys
    }

    fn render(&self, _f: &mut Frame, _area: Rect, _theme: &UiTheme) {
        // Real conversion: reads state via self.state.borrow() and
        // delegates to a panel renderer.
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.state.borrow().is_detail_open() {
            vec![
                ("C-n/C-p", "prev/next"),
                ("Enter/Esc", "back"),
                ("i", "close wiki"),
            ]
        } else {
            vec![
                ("C-n/C-p", "select"),
                ("Enter", "open"),
                ("i", "close wiki"),
            ]
        }
    }
}

/// Map-side view of wiki. Registered once at startup in `App`'s
/// painter list; drawn every frame regardless of whether
/// `WikiComponent` is on the compositor stack.
pub struct WikiPainter {
    state: WikiHandle,
}

impl WikiPainter {
    pub fn new(state: WikiHandle) -> Self {
        Self { state }
    }
}

impl Painter for WikiPainter {
    fn paint(&self, p: &mut MapPainter<'_>) {
        let state = self.state.borrow();
        let (primary, accent) = {
            let theme = p.theme();
            (theme.accent, theme.accent_alt)
        };
        for (i, a) in state.articles.iter().enumerate() {
            let fg = if i == state.selected { accent } else { primary };
            p.point(LonLat { lon: a.lon, lat: a.lat }, '●', fg);
        }
    }
}

/// **Prototype** `register` function that plugin::wiki would expose.
/// In the real conversion this lives in `src/plugin/wiki/mod.rs`
/// (not here) — kept in-file for the prototype so the whole shape
/// is visible at once. Note: `WikiState`, `WikiHandle`,
/// `WikiComponent`, and `WikiPainter` never escape this function —
/// the `Registrar` sees only `dyn Painter` + `dyn Fn() -> dyn
/// Component`, and `App` never sees them at all.
pub fn wiki_register_prototype(r: &mut Registrar) {
    let state: WikiHandle = Rc::new(RefCell::new(WikiState::default()));

    // Always-on map markers.
    r.add_painter(Box::new(WikiPainter::new(state.clone())));

    // `i` opens the panel. Spawn clones the handle so every push
    // shares the same persistent state.
    r.add_activation(Activation {
        code: KeyCode::Char('i'),
        modifiers: KeyModifiers::NONE,
        spawn: {
            let state = state.clone();
            Box::new(move || Box::new(WikiComponent::new(state.clone())))
        },
    });

    // Palette entry also spawns the same component.
    r.add_palette_entry(PaletteEntry {
        label: "Toggle wiki".to_string(),
        spawn: {
            let state = state.clone();
            Box::new(move || Box::new(WikiComponent::new(state.clone())))
        },
    });

    // `state` goes out of scope here, but the `Rc` clones held by
    // Painter + the two spawn closures keep it alive for the
    // lifetime of the app. Nothing leaks into `App`'s type.
}

// ── App-level wiring sketch (not instantiated, just for reference) ──
//
// App is **plugin-agnostic** — no plugin struct, no plugin handle
// ever appears as a field:
//
//     pub struct App {
//         // ... existing fields ...
//         compositor: Compositor,
//         painters: Vec<Box<dyn Painter>>,
//         activations: Vec<Activation>,
//         palette_entries: Vec<PaletteEntry>,
//         // no `wiki: WikiHandle`, no `search: SearchPlugin`, nothing.
//     }
//
// Composition root (main.rs or src/plugins.rs — NOT app/mod.rs) is
// the single file that names concrete plugins:
//
//     fn build_registrar(config: &Config) -> Registrar {
//         let mut r = Registrar::default();
//         plugin::search::register(config, &mut r);
//         plugin::wiki::register(config, &mut r);
//         plugin::palette::register(config, &mut r);
//         plugin::help::register(config, &mut r);
//         plugin::here::register(config, &mut r);
//         r
//     }
//
//     let registrar = build_registrar(&config);
//     let app = App::new(config, registrar);
//
// Key event path (handled in `App::run`):
//     let msgs = self.compositor.handle_event(key, &ctx);
//     // If compositor returned nothing, the bottom layer (map
//     // keymap + activation dispatch — see below) gets the event.
//     for msg in msgs { self.dispatch(msg); }
//
// Activation dispatch is handled by a **bottom-layer** component
// that owns `activations: Vec<Activation>`. When a key matches an
// Activation, it calls `spawn()` and pushes the result onto the
// compositor. This replaces `BackgroundResponder` +
// `PluginRegistry::activations` — the key→plugin map is just a
// `Vec<Activation>` on that one component, fed by the Registrar.
//
// Paint:
//     for p in &self.painters { p.paint(&mut map_painter); }
//     // followed by ratatui draw, during which Compositor::render
//     // paints on top.

// ── Known design gaps (to resolve before full refactor) ─────────────
//
// 1. `here` plugin — UI-less, fires a geoip job on palette activate.
//    Doesn't fit Component or Painter. Either (a) palette entry's
//    `spawn` returns a synthetic no-UI Component that runs the job
//    in `poll` and `Close`s itself, or (b) add `tasks: Vec<Box<dyn
//    Task>>` to Registrar for pure async jobs.
//
// 2. `palette::SwitchProvider` — today switches the active provider
//    without closing. Under compositor, either (a) the palette
//    Component mutates its internal provider on the `SwitchProvider`
//    result (same Component instance), or (b) `Close(vec![])` +
//    immediate push of a new palette Component with the new
//    provider. (a) is closer to current behaviour; (b) is more
//    uniform.
//
// 3. Footer hints integration — `Compositor::footer_hints()` reads
//    the stack top. When the stack is empty, footer falls back to
//    the bottom-layer component's hints (today's
//    `BackgroundResponder::footer_hints`).
