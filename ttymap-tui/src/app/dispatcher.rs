//! `Dispatcher` — the GoF Receiver for [`crate::UserCommand`].
//!
//! Owns the state that mutates in response to commands (map, lua,
//! compositor, theme, sidebar, overlay sink, cursor) and every
//! handler that touches that state. [`crate::app::App`] is the loop
//! driver above this layer: it drains the
//! [`crate::app::AppEvent`] bus, ratatui-draws each frame, and
//! forwards events here.
//!
//! `Dispatcher` is **ratatui-free** — only `App::render_into`
//! touches ratatui. App reaches into Dispatcher state exclusively
//! through methods (`is_running`, `context`, `dispatch`,
//! `draw_inputs`, …); fields are private. The engine ↔ shell
//! relationship is one binary's two halves.
//!
//! Phase 1 of GitHub issue #212 (Dispatcher extraction). The
//! struct lives next to `App` (its sole consumer); the previous
//! core/front directory experiment (Phase 4) was reverted in
//! favour of a flat layout — see `src/lib.rs` header.

use std::rc::Rc;
use std::time::Duration;

use crossterm::event::KeyEvent;
use log::debug;

use super::overlay::OverlayThrottle;
use super::sidebar::SidebarPolicy;
use crate::UserCommand;
use crate::compositor::op::Op;
use crate::compositor::{Compositor, Context};
use crate::event::{Event, EventBus};
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
    /// Shared with App + the Lua subsystem. Dispatcher handlers
    /// publish observable events on this directly, alongside the
    /// state mutation that produced them — no wrapper layer, no
    /// second match on `UserCommand` (see #334).
    bus: Rc<EventBus>,
    compositor: Compositor,
    sidebar: SidebarPolicy,
    cursor: Option<(u16, u16)>,
    overlay: OverlayThrottle,
}

impl Dispatcher {
    pub(super) fn new(
        theme_id: ThemeId,
        map: MapHandle,
        lua: LuaHandle,
        bus: Rc<EventBus>,
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
            bus,
            compositor,
            sidebar: SidebarPolicy::new(sidebar_width),
            cursor: None,
            overlay: OverlayThrottle::new(overlay_redraw_interval),
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
    pub(super) fn draw_inputs<'a>(
        &'a mut self,
        map_frame: Option<&'a MapFrame>,
        ctx: &'a Context,
    ) -> super::ui::DrawInputs<'a> {
        super::ui::DrawInputs {
            map_frame,
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
                Op::Publish(event) => self.bus.publish(event),
            }
        }
    }

    /// Deliver a key event to the compositor and apply any resulting
    /// ops. Called from `App::handle_input` for non-Ctrl-C key
    /// events.
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

    /// Execute a [`UserCommand`]. Each arm mutates the state it owns
    /// and, on the same beat, publishes the observable [`Event`]
    /// (if any) that corresponds to "this just happened". No separate
    /// notify pass, no per-arm helper that only one arm calls — the
    /// full behaviour of each command is readable in one place.
    ///
    /// `handle_resize` stays a private helper because three call
    /// sites share it (`Resize` arm here, `ToggleSidebar` arm here,
    /// `poll_compositor` auto-resize after a sidebar count change).
    /// `request_map_redraw` likewise — it's invoked from every arm
    /// that wants the next frame.
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
                    MapAction::Jump(ll) => self.bus.publish(Event::MapJumped(*ll)),
                    MapAction::SetZoom(z) => self.bus.publish(Event::MapZoomSet(*z)),
                    MapAction::FlyTo { center, zoom } => {
                        self.bus.publish(Event::MapFlewTo(*center, *zoom));
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
                self.bus
                    .publish(Event::ThemeChanged(new_id.name().to_string()));
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
        self.bus.publish(Event::Resized(cols, rows));
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
