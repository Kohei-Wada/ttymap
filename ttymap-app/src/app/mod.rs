//! App — central state hub + loop driver.
//!
//! Holds every piece of mutable app-level state (map handle, lua
//! handle, compositor, theme, sidebar, cursor, overlay, mouse
//! adapter, latest map frame) and the four event entry points that
//! mutate it: [`Self::dispatch`] (`UserCommand`), [`Self::accept_frame`]
//! (`FrameReady`), [`Self::handle_input`] (raw crossterm), and
//! [`Self::forward_external_event`] (cross-thread `Bus`).
//!
//! Two invariants govern the shape:
//! - **Single publish site.** Handlers `push` into `pending_events`
//!   instead of calling `bus.publish` directly. The drain in
//!   [`Self::publish_pending`] is the only place `bus.publish` runs —
//!   the single fan-out point for the program.
//! - **State stays here.** No sub-struct splits state by aspect; the
//!   coupling between theme / map / lua / overlay / sidebar makes
//!   shared mutable access unavoidable, so we keep one owner and
//!   discipline mutation through the four entry points.
//!
//! Historically the state half was extracted into a separate
//! `Dispatcher` struct (issue #212 Phase 1) but every feature added
//! since landed back on the state side, so App was a thin router
//! over a god-struct that was App in everything but name. Merging
//! them back is the honest shape.
//!
//! `main` is the composition root: it builds the bus, the channel,
//! and the off-thread subsystems, then hands them in.

// `AppEvent` lives in `ttymap-tui` (it's drained by `App::run` here
// but produced by the TUI-side input thread and consumed by the
// compositor's render path). Re-exported for existing call sites in
// this module.
pub use ttymap_tui::AppEvent;

pub mod frame_timer;
mod frame_widget;
mod overlay;
mod sidebar;
pub mod ui;

use std::io;
use std::rc::Rc;
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};

use self::overlay::OverlayThrottle;
use self::sidebar::SidebarPolicy;
use ttymap_config::Config;
pub use ttymap_config::KeybindingOverrides;
use ttymap_core::UserCommand;
use ttymap_core::event::{Event, EventBus};
use ttymap_engine::map::render::frame::MapFrame;
use ttymap_engine::map::state::MapState;
use ttymap_lua::{LuaHandle, LuaSubsystem};
use ttymap_tui::compositor::op::Op;
use ttymap_tui::compositor::{BaseLayer, Compositor, Context};
use ttymap_tui::input::{KeyMap, MouseAdapter};
use ttymap_tui::theme::{ThemeId, UiTheme};

use crate::engine_handle::EngineHandle;

pub struct App {
    /// UI-side mirror of the engine's viewport state. `dispatch`
    /// mutates this synchronously, then forwards the same
    /// `MapAction` to the engine via `self.map.send_action`. The
    /// child runs the identical transitions on the same inputs, so
    /// the two stay coherent by construction — Lua's same-tick
    /// getters (`ttymap.map:center()` etc.) read this mirror and
    /// never block on IPC.
    map_state: MapState,
    /// Pure IPC transport to the `ttymap engine-worker` subprocess.
    /// The App owns its own `MapState`; this handle just sends
    /// commands and pipes back frames.
    map: EngineHandle,
    running: bool,
    theme_id: ThemeId,
    ui_theme: UiTheme,
    lua: LuaHandle,
    compositor: Compositor,
    sidebar: SidebarPolicy,
    cursor: Option<(u16, u16)>,
    overlay: OverlayThrottle,
    mouse: MouseAdapter,
    /// Latest rendered map snapshot drained from the render thread.
    /// `None` until the first frame arrives. Updated by
    /// [`Self::accept_frame`] on every `AppEvent::FrameReady`.
    map_frame: Option<MapFrame>,
    /// Events accumulated since the last drain. Handlers `push` here
    /// rather than calling `bus.publish` directly so all fan-out
    /// happens at one place in [`Self::publish_pending`].
    pending_events: Vec<Event>,
    /// The Lua-agnostic pub/sub primitive. Only [`Self::publish_pending`]
    /// calls `publish` on it — the single fan-out site for the
    /// program.
    bus: Rc<EventBus>,
    /// Main event-loop wake interval. Derived from
    /// `ttymap.opt.runtime.poll_timeout_ms` at startup. `pub` getter
    /// because `main` reads it to align the input thread / frame
    /// timer cadences.
    poll_timeout: Duration,
}

impl App {
    /// Build the App.
    ///
    /// Composition root (`main`) builds every subsystem upstream and
    /// hands them in: the map subsystem as [`EngineHandle`] (running
    /// in a sibling subprocess — see #348), the Lua plugin subsystem
    /// as [`LuaSubsystem`] (already with the palette installed). App
    /// just consumes them — its only own work is wiring the compositor
    /// base layer and assembling its own fields.
    pub fn new(
        config: Config,
        keymap: KeyMap,
        theme_id: ThemeId,
        map_state: MapState,
        map: EngineHandle,
        builtin_activations: Vec<ttymap_tui::compositor::Activation>,
        lua: LuaSubsystem,
    ) -> Self {
        let LuaSubsystem {
            handle: lua,
            bus,
            ticks: _,
            registry,
            footer_hints,
        } = lua;

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top. BaseLayer takes an `ActivationIndex` trait
        // object — today backed by the Lua-side registry through
        // `LuaActivationIndex`, but the compositor itself stays
        // unaware that Lua is the source. Plugin
        // `KeybindHandle:remove()` updates are visible on the next
        // keypress because the wrapper holds a clone of the same
        // `LuaRegistryHandle`. Built-in activations (today: just `:`
        // for the palette) are kept in their own Vec so plugins
        // can't accidentally shadow host shortcuts.
        let activation_index: std::rc::Rc<dyn ttymap_tui::compositor::ActivationIndex> =
            std::rc::Rc::new(ttymap_lua::LuaActivationIndex::new(registry));
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(
            keymap,
            builtin_activations,
            activation_index,
            footer_hints,
        )));

        let ui_theme = UiTheme::from_palette(theme_id.palette());
        App {
            map_state,
            map,
            running: true,
            theme_id,
            ui_theme,
            lua,
            compositor,
            sidebar: SidebarPolicy::new(config.runtime.sidebar_width),
            cursor: None,
            overlay: OverlayThrottle::new(Duration::from_millis(config.runtime.overlay_redraw_ms)),
            mouse: MouseAdapter::default(),
            map_frame: None,
            pending_events: Vec::new(),
            bus,
            poll_timeout: Duration::from_millis(config.runtime.poll_timeout_ms),
        }
    }

    /// The configured idle wake-up interval — `main` reads this when
    /// spinning up the input thread / frame timer so they share the
    /// same cadence.
    pub fn poll_timeout(&self) -> Duration {
        self.poll_timeout
    }

    /// Drive the per-iteration event loop until [`UserCommand::Quit`]
    /// flips `running` off.
    ///
    /// Shape: drain queue → poll components → apply Lua ops →
    /// publish pending → render → throttle overlay redraw. `main`
    /// stays the composition root: it builds the bus, the channel,
    /// and the off-thread subsystems, then hands them in here as
    /// borrows.
    pub fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        event_rx: &std::sync::mpsc::Receiver<AppEvent>,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) -> io::Result<()> {
        // Kick off the very first render task so the terminal isn't
        // blank waiting for input. Goes direct to the render path
        // rather than through `dispatch` — the engine has no state
        // change to apply, just needs a frame.
        self.request_map_redraw();
        self.publish_pending();

        while self.running {
            // Park on the unified queue until any source produces an
            // event; drain any further buffered events non-blockingly
            // so a burst doesn't push the paint behind.
            match event_rx.recv() {
                Ok(event) => self.handle_event(event, event_tx),
                Err(_) => break,
            }
            while let Ok(event) = event_rx.try_recv() {
                self.handle_event(event, event_tx);
            }

            // Component poll: any handler-returned `Op`s apply through
            // the accumulator; any `Op::Publish` lands in
            // `pending_events` and ships out at the next drain.
            self.poll_compositor();

            // Drain Lua-enqueued ops *before* render so that ops
            // emitted by handler / palette / keybind callbacks during
            // event handling apply this frame. on_tick-emitted ops
            // fire during `render_into` below — those land in the
            // buffer and drain at the start of the *next* iteration's
            // `poll_compositor`, with the same one-frame visibility
            // lag as the prior CloseFlag-via-poll design.
            self.apply_lua_ops();

            // Single bus-publish site for the entire program. Every
            // `pending_events` push (dispatch arms, accept_frame,
            // forward_external_event, Op::Publish in apply_ops) ships
            // out here.
            self.publish_pending();

            // Render a frame. Inside `ui::draw`, the per-frame Lua
            // `tick` event fires against the live MapApi.
            self.render_into(terminal)?;

            // If plugin `on_tick` callbacks pushed polylines, throttle
            // the redraw request to the configured interval.
            if self.overlay.should_redraw() {
                self.request_map_redraw();
            }
        }

        Ok(())
    }

    /// Route one [`AppEvent`] into the right entry point. Each arm is
    /// a one-line forward; the actual state mutation lives in the
    /// invoked method.
    fn handle_event(&mut self, event: AppEvent, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            AppEvent::Command(msg) => self.dispatch(msg),
            AppEvent::FrameReady(frame) => self.accept_frame(frame),
            AppEvent::Input(input) => self.handle_input(input, event_tx),
            // `Wake` exists purely to unblock `event_rx.recv()`. The
            // per-iteration draw + overlay-redraw rate-check below
            // already does whatever per-frame work is needed; no
            // extra handler logic belongs here. Distinct from the
            // Lua-side `"tick"` event which fires from inside draw.
            AppEvent::Wake => {}
            // Cross-thread producers route through here so their
            // publish lands in the same accumulator dispatch-produced
            // events do — preserving the single-publish-site invariant.
            AppEvent::Bus(bus_event) => self.forward_external_event(bus_event),
        }
    }

    /// Drain every [`Event`] accumulated since the last drain and
    /// publish each onto the bus. The single fan-out point.
    fn publish_pending(&mut self) {
        for ev in std::mem::take(&mut self.pending_events) {
            self.bus.publish(ev);
        }
    }

    /// Mirror the latest [`MapFrame`] into the Lua-readable cell and
    /// update the App-visible cache used by `render_into`.
    fn accept_frame(&mut self, frame: MapFrame) {
        self.lua.set_current_frame(frame.clone());
        self.map_frame = Some(frame);
    }

    /// Forward a bus event published by an off-thread producer. The
    /// cross-thread `AppEvent::Bus` branch routes through here so the
    /// publish lands in the same accumulator dispatch-produced
    /// events do.
    fn forward_external_event(&mut self, event: Event) {
        self.pending_events.push(event);
    }

    /// Translate a raw [`crossterm::event::Event`] into the right
    /// downstream action. Key events route through the focus stack
    /// directly ([`Self::handle_key_event`]); resize / mouse become
    /// [`UserCommand`]s pushed back on the App-level queue so the
    /// same handler path applies whether the trigger was the terminal
    /// or a Lua palette command. Ctrl-C is the single hard-coded
    /// host shortcut; everything else is keymap-defined.
    fn handle_input(
        &mut self,
        event: CrosstermEvent,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) {
        match event {
            CrosstermEvent::Key(key_event) => {
                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && key_event.code == KeyCode::Char('c')
                {
                    info!("Ctrl-C received, quitting");
                    let _ = event_tx.send(AppEvent::Command(UserCommand::Quit));
                } else {
                    debug!("key event: {:?}", key_event.code);
                    self.handle_key_event(key_event);
                }
            }
            CrosstermEvent::Resize(cols, rows) => {
                info!("resize: {}x{}", cols, rows);
                let _ = event_tx.send(AppEvent::Command(UserCommand::Resize(cols, rows)));
            }
            CrosstermEvent::Mouse(mouse) => {
                for msg in self.mouse.translate(mouse) {
                    let _ = event_tx.send(AppEvent::Command(msg));
                }
            }
            _ => {}
        }
    }

    /// Deliver a key event to the compositor and apply any resulting
    /// ops. Called from [`Self::handle_input`] for non-Ctrl-C keys.
    fn handle_key_event(&mut self, key: KeyEvent) {
        let ctx = self.context();
        let ops = self.compositor.handle_key(key, &ctx);
        self.apply_ops(ops);
    }

    /// Drive a single `compositor.poll` pass plus the sidebar
    /// auto-open observation. Called once per loop iteration.
    fn poll_compositor(&mut self) {
        let ctx = self.context();
        let ops = self.compositor.poll(&ctx);
        self.apply_ops(ops);

        let count = self.compositor.sidebar_component_count();
        if self.sidebar.observe_count(count) {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            self.handle_resize(cols, rows);
        }
    }

    /// Drain queued Lua-side ops and apply them. App calls this once
    /// per loop iteration after event handling, before render.
    fn apply_lua_ops(&mut self) {
        let ops = self.lua.drain_ops();
        self.apply_ops(ops);
    }

    /// Apply a batch of [`Op`]s from any source (Lua callbacks,
    /// component handlers, component polls). All converge on this
    /// single applier.
    fn apply_ops(&mut self, ops: Vec<Op>) {
        for op in ops {
            match op {
                Op::Push { id, component } => {
                    self.compositor.push_with_id(id, component);
                }
                Op::Close(id) => self.compositor.close_by_id(id),
                Op::Command(cmd) => self.dispatch(cmd),
                Op::Publish(event) => self.pending_events.push(event),
            }
        }
    }

    /// Execute a [`UserCommand`]. Each arm mutates the state it
    /// owns. `handle_resize` stays a private helper because three
    /// call sites share it.
    fn dispatch(&mut self, msg: UserCommand) {
        match msg {
            UserCommand::Map(action) => {
                if self.map_state.process_action(&action) {
                    self.map.send_action(&action);
                    self.request_map_redraw();
                }
            }
            UserCommand::Quit => {
                debug!("UserCommand::Quit — stopping event loop");
                self.running = false;
            }
            UserCommand::SetTheme(new_id) => {
                self.theme_id = new_id;
                self.ui_theme = UiTheme::from_palette(new_id.palette());
                self.map.set_theme(new_id);
                self.request_map_redraw();
            }
            UserCommand::CursorMoved(col, row) => {
                self.cursor = Some((col, row));
            }
            UserCommand::CycleFocus(forward) => {
                self.compositor.cycle(forward);
            }
            UserCommand::Resize(cols, rows) => self.handle_resize(cols, rows),
            UserCommand::ToggleSidebar => {
                self.sidebar.toggle();
                // The visible map area shrinks/expands — re-run the
                // resize path so the render thread allocates the right
                // canvas size.
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                self.handle_resize(cols, rows);
            }
            UserCommand::SetLabelsVisible(visible) => {
                self.map.set_labels_visible(visible);
                self.request_map_redraw();
            }
        }
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        let map_cols = self.sidebar.effective_map_cols(cols);
        self.map_state.resize(map_cols, rows);
        self.map.send_resize(map_cols, rows);
        self.request_map_redraw();
    }

    fn request_map_redraw(&mut self) {
        // Refresh the Lua-side view mirror in the same beat as
        // queueing the next render task — Lua plugins read
        // `ttymap.map:center()` / `:zoom()` from these mirrors and
        // expect them to reflect the view about to be drawn.
        self.lua
            .sync_view(self.map_state.center(), self.map_state.zoom());
        let overlays = self.overlay.drain();
        self.map.request_redraw(overlays);
    }

    /// Build the [`Context`] snapshot read by component hooks.
    fn context(&self) -> Context {
        Context {
            theme_id: self.theme_id,
            cursor: self.cursor,
        }
    }

    /// Single per-iteration draw. The `tick` bus event fires from
    /// inside `ui::draw` against the live `MapApi` (see `ui::draw`).
    fn render_into(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        let ctx = self.context();
        let inputs = ui::DrawInputs {
            map_frame: self.map_frame.as_ref(),
            compositor: &self.compositor,
            lua: &self.lua,
            theme: &self.ui_theme,
            ctx: &ctx,
            overlay_sink: self.overlay.sink_mut(),
            sidebar_open: self.sidebar.open,
            sidebar_width: self.sidebar.width,
        };
        terminal.draw(|f| ui::draw(f, inputs))?;
        Ok(())
    }
}
