//! Frontend layer — receives [`AppEvent`]s, mutates state, and
//! draws.
//!
//! [`Frontend`] is the sole **Receiver** in the GoF Command pattern:
//! every invoker (keymap, palette, plugins, mouse adapter, render
//! thread, input thread, frame timer) emits onto the unified
//! [`AppEvent`] bus, and only `Frontend::dispatch` executes the
//! resulting `UserIntent`s. Component hooks express intent through
//! `Window::emit(msg)` which routes onto the same bus — no
//! synchronous "return `Vec<UserIntent>`" path remains.
//!
//! `Frontend` doesn't own subsystems. Threads (render / input /
//! frame timer), the bus, and the channel are constructed by `main`
//! at the composition root and handed in. The Frontend just runs
//! the per-iteration handler the loop calls into.
//!
//! Focus/modal state lives on [`Compositor`] — a stack of
//! [`Component`]s that replaced the old `FocusManager` + `Plugin`
//! trilogy. Components render map overlays through
//! `Component::paint_on_map`. The compositor never names a concrete
//! plugin type — it carries only `Box<dyn Component>`s populated by
//! the Lua dispatcher at composition time.

pub(crate) mod compositor;
pub mod event;
pub mod frame_timer;
pub mod intent;
pub mod palette;
pub mod ui;

pub use event::AppEvent;
pub use intent::UserIntent;

use std::io;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::config::Config;
use crate::frontend::compositor::{BaseLayer, Compositor, Context};
pub use crate::input::KeybindingOverrides;
use crate::input::{KeyMap, MouseAdapter};
use crate::lua::LuaHandle;
use crate::lua::LuaSubsystem;
use crate::map::Action;
use crate::map::MapHandle;
use crate::map::render::frame::MapFrame;
use crate::theme::ThemeId;
use crate::theme::UiTheme;

pub struct Frontend {
    /// Map subsystem handle: dispatch state + render-task sender +
    /// theme id + attribution. Built by [`crate::map::build`] in
    /// `main` and handed in. The owning `RenderHandle` lives in
    /// `main`'s scope as a peer subsystem alongside `InputHandle`
    /// and `FrameTimer`.
    map: MapHandle,
    /// Runtime handle to the Lua subsystem. Encapsulates the event
    /// bus and per-plugin host channels so Frontend never names them
    /// directly — every Lua-side interaction goes through semantic
    /// methods (`notify_*`, `tick`, `sync_view`, `drain_pushes`).
    lua: LuaHandle,
    /// Latest rendered map snapshot drained from the render thread.
    /// `None` until the first frame arrives. Owned here directly —
    /// no UiState wrapper now that built-in chrome lives in plugins.
    map_frame: Option<MapFrame>,
    mouse: MouseAdapter,
    compositor: Compositor,
    ui_theme: UiTheme,
    /// Ephemeral polyline overlays pushed by Lua plugins during the
    /// current frame's `on_tick` pass. Drained into the next
    /// `RenderTask::Draw` immediately after `ui::draw` returns — so
    /// the Lua side fire-and-forgets every frame and the render thread
    /// always gets the freshest set.
    overlay_sink: Vec<crate::map::render::overlay::UserPolyline>,
    /// Latest mouse cursor position in absolute terminal cells.
    /// `None` until the first mouse event arrives (or always, on
    /// terminals without mouse support). Surfaced to components via
    /// [`Context`](crate::frontend::compositor::Context) so plugins can build
    /// cursor-aware overlays.
    cursor: Option<(u16, u16)>,
    /// Timestamp of the last overlay-driven `request_map_redraw`. Used
    /// to rate-limit redraws when a plugin pushes polyline overlays
    /// every tick — without this, each push triggers a full tile re-
    /// render at the main loop's ~60Hz cadence (`event::poll(16ms)`),
    /// which is wasted work since tile data does not change between
    /// frames. Throttling to ~30Hz halves render-thread CPU while
    /// keeping animation visually smooth. User-event-driven redraws
    /// (pan, zoom, resize, theme change) bypass this check and fire
    /// immediately.
    last_overlay_redraw: std::time::Instant,
    /// Main event-loop wake interval. Derived from
    /// `ttymap.opt.runtime.poll_timeout_ms` at startup.
    poll_timeout: std::time::Duration,
    /// Minimum interval between overlay-driven redraws. Derived from
    /// `ttymap.opt.runtime.overlay_redraw_ms` at startup.
    overlay_redraw_interval: std::time::Duration,
}

impl Frontend {
    /// Build the Frontend.
    ///
    /// Composition root (`main`) builds every subsystem upstream and
    /// hands them in: the map subsystem as [`MapHandle`], the Lua
    /// plugin subsystem as [`LuaSubsystem`] (already with the palette
    /// installed). Frontend just consumes them — its only own work
    /// is wiring the compositor base layer, deriving the UI theme,
    /// and storing the per-iteration state.
    pub fn new(config: Config, keymap: KeyMap, map: MapHandle, lua: LuaSubsystem) -> Self {
        let LuaSubsystem {
            handle: lua,
            activations,
            plugin_hints,
            // `palette_entries` was already drained by
            // `palette::install` from main; nothing left for
            // Frontend to consume.
            palette_entries: _,
        } = lua;

        let ui_theme = UiTheme::from_palette(map.theme_id().palette());

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(keymap, activations, plugin_hints)));

        Frontend {
            map,
            lua,
            map_frame: None,
            mouse: MouseAdapter::default(),
            compositor,
            ui_theme,
            cursor: None,
            overlay_sink: Vec::new(),
            last_overlay_redraw: std::time::Instant::now(),
            poll_timeout: std::time::Duration::from_millis(config.runtime.poll_timeout_ms),
            overlay_redraw_interval: std::time::Duration::from_millis(
                config.runtime.overlay_redraw_ms,
            ),
        }
    }

    /// The configured idle wake-up interval — `main` reads this when
    /// spinning up the input thread / frame timer so they share the
    /// same cadence.
    pub fn poll_timeout(&self) -> std::time::Duration {
        self.poll_timeout
    }

    /// Drive the per-iteration event loop until the map state
    /// requests shutdown.
    ///
    /// The frontend owns the iteration shape (housekeeping → drain
    /// queue → poll components → render → throttle overlay redraw)
    /// because the ordering between those steps is a frontend
    /// concern, not a wiring concern. `main` stays the composition
    /// root: it builds the bus, the channel, and the off-thread
    /// subsystems, then hands them in here as borrows.
    pub fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        event_rx: &std::sync::mpsc::Receiver<AppEvent>,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) -> io::Result<()> {
        self.dispatch_initial_redraw();

        while self.is_running() {
            // Per-plugin housekeeping before the event drain — Lua
            // plugins queued via `ttymap.api.window.open` reach the
            // compositor before this iteration's `poll_compositor`.
            self.refresh_lua_host_state();

            // Park on the unified bus until any source produces an
            // event; drain any further buffered events non-blockingly
            // so a burst doesn't push the paint behind.
            match event_rx.recv() {
                Ok(event) => self.handle_event(event, event_tx),
                Err(_) => break,
            }
            while let Ok(event) = event_rx.try_recv() {
                self.handle_event(event, event_tx);
            }

            // Component poll: any `win.emit(msg)` inside fires onto
            // the bus directly. Same-iteration `try_recv` ran above
            // already; an emission here will be picked up next
            // iteration.
            self.poll_compositor(event_tx);

            // Render a frame. Inside `ui::draw`, the per-frame Lua
            // `tick` event fires against the live MapApi.
            self.render_into(terminal)?;

            // If plugin `on_tick` callbacks pushed polylines, throttle
            // the redraw request to the configured interval.
            self.tick_overlay_redraw();
        }

        Ok(())
    }

    /// Whether the map state machine still wants the loop to keep
    /// running. Checked at the top of each `run` iteration.
    fn is_running(&self) -> bool {
        self.map.is_running()
    }

    /// Drain every Lua plugin's setup-state receivers once.
    ///
    /// Before draining, refreshes the host-shared `center` / `zoom`
    /// cells each plugin's `ttymap.map:center()` / `:zoom()` reads
    /// from. Doing this once at the App level — instead of from
    /// inside each `LuaWindowComponent` dispatch — means the values
    /// are correct for *every* callback path (palette invoke,
    /// register_keybind, on_tick), not just paths that go through
    /// an active window.
    ///
    /// Per-plugin housekeeping: refresh `center` / `zoom` mirrors that
    /// `ttymap.map:center()` / `:zoom()` read, and drain each plugin's
    /// `push_rx` so any component queued via `ttymap.api.window.open`
    /// / `palette.open` reaches the compositor stack before the
    /// current tick's `compositor.poll`. Stays per-plugin because
    /// `Box<dyn Component>` is `!Send` and the channel must remain on
    /// the main thread.
    ///
    /// Intent flow (`map:jump`, `frame.export`, …) is **not** drained
    /// here — those reach the App through the unified `event_rx`.
    fn refresh_lua_host_state(&mut self) {
        let center = self.map.center();
        let zoom = self.map.zoom();
        self.lua.sync_view(center, zoom);
        // Disjoint borrows: `&self.lua` for the iterator, `&mut
        // self.compositor` for the push side.
        let lua = &self.lua;
        let compositor = &mut self.compositor;
        lua.drain_pushes(|component| compositor.push(component));
    }

    /// Apply one event drained off the unified queue. Each variant
    /// has a small fixed handler and the work is bounded — long Lua
    /// callbacks notwithstanding, the loop never sits inside this
    /// for more than the time a single dispatch needs.
    ///
    /// Two-stage shape per variant: **execute** (state mutation —
    /// `dispatch`, `map_frame = ...`, etc.) followed by **notify**
    /// (broadcast a post-effect notification on the Lua event bus).
    /// Subscribers (Lua plugins via `ttymap.on_event`) only see the
    /// notification path; they read the resulting state through the
    /// usual `ttymap.map:*` accessors. Intent flow stays single-
    /// executor; the bus is observation only.
    fn handle_event(&mut self, event: AppEvent, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            AppEvent::Intent(msg) => {
                let snapshot = msg.clone();
                self.dispatch(msg);
                self.notify_post_intent(&snapshot);
            }
            AppEvent::FrameReady(frame) => {
                self.map_frame = Some(frame);
                self.lua.notify_frame_ready();
            }
            AppEvent::Input(input) => self.handle_input(input, event_tx),
            AppEvent::LuaIntent(intent) => {
                // Translate Lua-originated intents to the App's own
                // imperative vocabulary. The lua subsystem doesn't
                // import `UserIntent` / `Action`; the boundary lives
                // here.
                let msg = lua_intent_to_user_intent(intent);
                let snapshot = msg.clone();
                self.dispatch(msg);
                self.notify_post_intent(&snapshot);
            }
            // `Wake` exists purely to unblock `event_rx.recv()`. The
            // per-iteration draw + overlay-redraw rate-check below
            // already does whatever per-frame work is needed; no
            // extra handler logic belongs here. Distinct from the
            // Lua-side `"tick"` event which fires from inside draw.
            AppEvent::Wake => {}
        }
    }

    /// Broadcast post-effect notifications for the variants that
    /// plugins want to observe. Skips noisy / internal intents
    /// (`PanCells`, `CursorMoved`, `CycleFocus`, etc.) so the bus
    /// surface stays meaningful — bus events are "something
    /// observable happened to the app", not "every state mutation".
    fn notify_post_intent(&self, msg: &UserIntent) {
        match msg {
            UserIntent::Map(Action::Jump(ll)) => self.lua.notify_map_jumped(*ll),
            UserIntent::Map(Action::SetZoom(z)) => self.lua.notify_map_zoom_set(*z),
            UserIntent::Map(Action::FlyTo { center, zoom }) => {
                self.lua.notify_map_flew_to(*center, *zoom);
            }
            UserIntent::SetTheme(new_id) => self.lua.notify_theme_changed(new_id.name()),
            UserIntent::Resize(cols, rows) => self.lua.notify_resized(*cols, *rows),
            UserIntent::ExportFrame => self.lua.notify_frame_exported(),
            // Noisy or internal — `PanCells`, `ZoomAt`, `CursorMoved`,
            // `CycleFocus`, the discrete `Pan*` keymap actions, and
            // `Quit` (the App is tearing down anyway) are deliberately
            // not broadcast. Adding them later is one match arm.
            _ => {}
        }
    }

    /// Classify a raw terminal event and dispatch downstream messages.
    /// Same logic as the prior inline `crossterm::event::poll` block —
    /// just relocated so it can run from the unified-queue drain.
    fn handle_input(&mut self, event: Event, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            Event::Key(key_event) => {
                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && key_event.code == KeyCode::Char('c')
                {
                    info!("Ctrl-C received, quitting");
                    let _ = event_tx.send(AppEvent::Intent(UserIntent::Map(Action::Quit)));
                } else {
                    debug!("key event: {:?}", key_event.code);
                    let ctx = self.context();
                    self.compositor.handle_event(key_event, &ctx, event_tx);
                }
            }
            Event::Resize(cols, rows) => {
                info!("resize: {}x{}", cols, rows);
                let _ = event_tx.send(AppEvent::Intent(UserIntent::Resize(cols, rows)));
            }
            Event::Mouse(mouse) => {
                for msg in self.mouse.translate(mouse) {
                    let _ = event_tx.send(AppEvent::Intent(msg));
                }
            }
            _ => {}
        }
    }

    fn context(&self) -> Context {
        Context {
            theme_id: self.map.theme_id(),
            cursor: self.cursor,
        }
    }

    /// Drive a single `compositor.poll` pass. Components emitting
    /// intents through `Window::emit` route directly onto the bus —
    /// the run loop's same-iteration `try_recv` picks them up.
    fn poll_compositor(&mut self, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        let ctx = self.context();
        self.compositor.poll(&ctx, event_tx);
    }

    /// Single per-iteration draw. The `tick` bus event fires from
    /// inside `ui::draw` against the live `MapApi` (see `ui::draw`).
    fn render_into(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        let ctx = self.context();
        // Field-disjoint borrows so the closure can hold `&self.X`
        // alongside `&mut self.overlay_sink`.
        let map_frame = self.map_frame.as_ref();
        let compositor = &self.compositor;
        let lua = &self.lua;
        let ui_theme = &self.ui_theme;
        let overlay_sink = &mut self.overlay_sink;
        terminal.draw(|f| {
            crate::frontend::ui::draw(f, map_frame, compositor, lua, ui_theme, &ctx, overlay_sink)
        })?;
        Ok(())
    }

    /// After draw: if Lua plugins pushed polylines into
    /// `overlay_sink` during `on_tick` and the throttle interval
    /// has elapsed, queue a fresh `RenderTask::Draw` so the next
    /// frame carries them.
    fn tick_overlay_redraw(&mut self) {
        if !self.overlay_sink.is_empty() {
            let now = std::time::Instant::now();
            if now.duration_since(self.last_overlay_redraw) >= self.overlay_redraw_interval {
                self.request_map_redraw();
                self.last_overlay_redraw = now;
            }
        }
    }

    /// Initial dispatch fired by `run` right after entering the
    /// loop — kicks off the very first render task so the terminal
    /// isn't blank waiting for input.
    fn dispatch_initial_redraw(&mut self) {
        self.dispatch(UserIntent::Map(Action::Redraw));
    }

    fn dispatch(&mut self, msg: UserIntent) {
        match msg {
            UserIntent::Map(action) => {
                if self.map.apply_action(&action) {
                    self.request_map_redraw();
                }
            }
            UserIntent::SetTheme(new_id) => self.switch_theme(new_id),
            UserIntent::CursorMoved(col, row) => {
                self.cursor = Some((col, row));
            }
            UserIntent::CycleFocus(forward) => {
                self.compositor.cycle(forward);
            }
            UserIntent::Resize(cols, rows) => self.handle_resize(cols, rows),
            UserIntent::ExportFrame => self.export_current_frame(),
        }
    }

    fn export_current_frame(&self) {
        let Some(frame) = self.map_frame.as_ref() else {
            log::warn!("export: no frame to write yet");
            return;
        };
        let Some(dirs) = directories::ProjectDirs::from("", "", "ttymap") else {
            log::warn!("export: no ProjectDirs available");
            return;
        };
        let dir = dirs.data_dir().join("exports");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("export: mkdir {} failed: {e}", dir.display());
            return;
        }
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!(
            "ttymap-z{}-{:.4}-{:.4}-{}.ans",
            frame.zoom.floor() as i32,
            frame.center.lat,
            frame.center.lon,
            stamp
        );
        let path = dir.join(&name);
        match std::fs::write(&path, frame.to_ansi()) {
            Ok(()) => log::info!("export: wrote {}", path.display()),
            Err(e) => log::warn!("export: write {} failed: {e}", path.display()),
        }
    }

    fn switch_theme(&mut self, new_id: ThemeId) {
        self.map.set_theme(new_id);
        self.ui_theme = UiTheme::from_palette(new_id.palette());
        self.request_map_redraw();
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.map.handle_resize(cols, rows);
        self.request_map_redraw();
    }

    fn request_map_redraw(&mut self) {
        let overlays = std::mem::take(&mut self.overlay_sink);
        self.map.request_redraw(overlays);
    }
}

/// Translate a Lua-originated [`LuaIntent`] into the App's own
/// [`UserIntent`] vocabulary. Lives in the frontend (not the lua
/// module) because the lua module deliberately doesn't import
/// `UserIntent` / `Action` — the lua subsystem brokers events; this
/// function is the boundary that interprets them.
fn lua_intent_to_user_intent(intent: crate::lua::intent::LuaIntent) -> UserIntent {
    use crate::lua::intent::LuaIntent;
    match intent {
        LuaIntent::MapJump(ll) => UserIntent::Map(Action::Jump(ll)),
        LuaIntent::MapZoomSet(z) => UserIntent::Map(Action::SetZoom(z)),
        LuaIntent::MapFlyTo { center, zoom } => UserIntent::Map(Action::FlyTo { center, zoom }),
        LuaIntent::FrameExport => UserIntent::ExportFrame,
    }
}
