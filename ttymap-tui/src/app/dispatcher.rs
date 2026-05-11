//! `Dispatcher` — sole owner of mutable App state.
//!
//! Receives every [`crate::app::AppEvent`] variant App drains off
//! the unified queue, mutates the state it owns (map, lua, compositor,
//! theme, sidebar, overlay, cursor, mouse adapter, latest map frame),
//! and **accumulates** the observable [`Event`]s each mutation
//! produces into `pending_events`. App drains the buffer and
//! publishes once per loop iteration — a single bus.publish call
//! site for the entire program.
//!
//! No event publishing happens inside this module: Dispatcher does
//! not hold an `EventBus` reference. That keeps state mutation
//! (Dispatcher's job) and event fan-out (App's job, via the bus it
//! owns) in distinct hands.
//!
//! `Dispatcher` is **ratatui-free** — only `App::render_into` touches
//! ratatui. App reaches Dispatcher state through methods only;
//! fields are private. The engine ↔ shell relationship is one
//! binary's two halves.

use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};

use super::AppEvent;
use super::overlay::OverlayThrottle;
use super::sidebar::SidebarPolicy;
use crate::UserCommand;
use crate::compositor::op::Op;
use crate::compositor::{Compositor, Context};
use crate::event::Event;
use crate::input::MouseAdapter;
use crate::lua::LuaHandle;
use crate::theme::ThemeId;
use crate::theme::UiTheme;
use ttymap_engine::map::MapAction;
use ttymap_engine::map::MapHandle;
use ttymap_engine::map::render::frame::MapFrame;

pub(super) struct Dispatcher {
    map: MapHandle,
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
    /// [`Self::accept_frame`] on every `AppEvent::FrameReady` and
    /// borrowed back by `App::render_into` through [`Self::draw_inputs`].
    map_frame: Option<MapFrame>,
    /// Events accumulated since the last drain. Handlers `push` here
    /// rather than calling `bus.publish` directly so all fan-out
    /// happens at one place in `App::run`'s loop (single canonical
    /// publish site — see crate doc).
    pending_events: Vec<Event>,
}

impl Dispatcher {
    pub(super) fn new(
        theme_id: ThemeId,
        map: MapHandle,
        lua: LuaHandle,
        compositor: Compositor,
        sidebar_width: u16,
        overlay_redraw_interval: Duration,
    ) -> Self {
        let ui_theme = UiTheme::from_palette(theme_id.palette());
        Self {
            map,
            running: true,
            theme_id,
            ui_theme,
            lua,
            compositor,
            sidebar: SidebarPolicy::new(sidebar_width),
            cursor: None,
            overlay: OverlayThrottle::new(overlay_redraw_interval),
            mouse: MouseAdapter::default(),
            map_frame: None,
            pending_events: Vec::new(),
        }
    }

    /// Whether the event loop should keep running — flipped off by
    /// [`UserCommand::Quit`]. Checked at the top of each `App::run`
    /// iteration.
    pub(super) fn is_running(&self) -> bool {
        self.running
    }

    /// Initial dispatch fired by `App::run` right after entering the
    /// loop — kicks off the very first render task so the terminal
    /// isn't blank waiting for input.
    pub(super) fn dispatch_initial_redraw(&mut self) {
        self.dispatch(UserCommand::Map(MapAction::Redraw));
    }

    /// Take every [`Event`] accumulated since the last drain. App
    /// calls this after each dispatcher invocation and publishes
    /// the batch onto its bus.
    pub(super) fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending_events)
    }

    /// Build the [`Context`] snapshot read by component hooks.
    pub(super) fn context(&self) -> Context {
        Context {
            theme_id: self.theme_id,
            cursor: self.cursor,
        }
    }

    /// Bundle the per-frame borrows ratatui's `terminal.draw` closure
    /// needs into a single [`super::ui::DrawInputs`] so `App::render_into`
    /// stays a one-liner. Field-disjoint borrows (immut compositor /
    /// lua / theme + mut overlay sink) live behind this method
    /// instead of leaking the dispatcher's internals to App.
    pub(super) fn draw_inputs<'a>(&'a mut self, ctx: &'a Context) -> super::ui::DrawInputs<'a> {
        super::ui::DrawInputs {
            map_frame: self.map_frame.as_ref(),
            compositor: &self.compositor,
            lua: &self.lua,
            theme: &self.ui_theme,
            ctx,
            overlay_sink: self.overlay.sink_mut(),
            sidebar_open: self.sidebar.open,
            sidebar_width: self.sidebar.width,
        }
    }

    /// Drain queued Lua-side ops and apply them. App calls this once
    /// per loop iteration after event handling, before render.
    pub(super) fn apply_lua_ops(&mut self) {
        let ops = self.lua.drain_ops();
        self.apply_ops(ops);
    }

    /// Apply a batch of [`Op`]s from any source (Lua callbacks,
    /// component handlers, component polls). All converge on this
    /// single applier.
    pub(super) fn apply_ops(&mut self, ops: Vec<Op>) {
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

    /// Deliver a key event to the compositor and apply any resulting
    /// ops. Called from [`Self::handle_input`] for non-Ctrl-C keys.
    pub(super) fn handle_key_event(&mut self, key: KeyEvent) {
        let ctx = self.context();
        let ops = self.compositor.handle_key(key, &ctx);
        self.apply_ops(ops);
    }

    /// Drive a single `compositor.poll` pass plus the sidebar
    /// auto-open observation. Called once per loop iteration.
    pub(super) fn poll_compositor(&mut self) {
        let ctx = self.context();
        let ops = self.compositor.poll(&ctx);
        self.apply_ops(ops);

        let count = self.compositor.sidebar_component_count();
        if self.sidebar.observe_count(count) {
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            self.handle_resize(cols, rows);
        }
    }

    /// `true` when the overlay sink is non-empty AND the throttle
    /// interval has elapsed. App calls this after `render_into` and,
    /// on `true`, requests a fresh map redraw to flush queued
    /// polylines.
    pub(super) fn overlay_should_redraw(&mut self) -> bool {
        self.overlay.should_redraw()
    }

    /// Mirror the latest [`MapFrame`] into the Lua-readable cell,
    /// update the App-visible cache used by `render_into`, and queue
    /// `Event::FrameReady`. The Lua mirror is written before the
    /// event is enqueued so a `frame_ready` subscriber that
    /// immediately calls `ttymap.api.frame.to_ansi()` sees the new
    /// frame, not the previous one.
    pub(super) fn accept_frame(&mut self, frame: MapFrame) {
        self.lua.set_current_frame(frame.clone());
        self.map_frame = Some(frame);
        self.pending_events.push(Event::FrameReady);
    }

    /// Forward a bus event published by an off-thread producer. The
    /// cross-thread `AppEvent::Bus` branch routes through here so
    /// the publish lands in the same accumulator App drains for
    /// dispatch-produced events.
    pub(super) fn forward_external_event(&mut self, event: Event) {
        self.pending_events.push(event);
    }

    /// Translate a raw [`crossterm::event::Event`] into the right
    /// downstream action. Key events route through the focus stack
    /// directly (`handle_key_event`); resize / mouse become
    /// [`UserCommand`]s pushed back on the App-level queue so the
    /// same handler path applies whether the trigger was the terminal
    /// or a Lua palette command. Ctrl-C is the single hard-coded
    /// host shortcut; everything else is keymap-defined.
    pub(super) fn handle_input(
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

    /// Execute a [`UserCommand`]. Each arm mutates the state it owns
    /// and `push`es the observable [`Event`] (if any) into
    /// `pending_events`. No `bus.publish` call lives inside this
    /// module — App drains the buffer and publishes once per
    /// iteration. `handle_resize` stays a private helper because
    /// three call sites share it.
    ///
    /// `PanCells`, `ZoomAt`, `CursorMoved`, `CycleFocus`, the discrete
    /// `Pan*` keymap actions, and `Quit` are deliberately not
    /// broadcast — they're either noisy or internal.
    pub(super) fn dispatch(&mut self, msg: UserCommand) {
        match msg {
            UserCommand::Map(action) => {
                if self.map.apply_action(&action) {
                    self.request_map_redraw();
                }
                match &action {
                    MapAction::Jump(ll) => self.pending_events.push(Event::MapJumped(*ll)),
                    MapAction::SetZoom(z) => self.pending_events.push(Event::MapZoomSet(*z)),
                    MapAction::FlyTo { center, zoom } => {
                        self.pending_events.push(Event::MapFlewTo(*center, *zoom));
                    }
                    _ => {}
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
                self.pending_events
                    .push(Event::ThemeChanged(new_id.name().to_string()));
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
        self.map.handle_resize(map_cols, rows);
        self.pending_events.push(Event::Resized(cols, rows));
        self.request_map_redraw();
    }

    pub(super) fn request_map_redraw(&mut self) {
        // Refresh the Lua-side view mirror in the same beat as
        // queueing the next render task — Lua plugins read
        // `ttymap.map:center()` / `:zoom()` from these mirrors and
        // expect them to reflect the view about to be drawn.
        self.lua.sync_view(self.map.center(), self.map.zoom());
        let overlays = self.overlay.drain();
        self.map.request_redraw(overlays);
    }
}
