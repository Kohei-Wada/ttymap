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
//! touches ratatui. Field access from the App side is via
//! `pub(super)` fields rather than accessor methods; the engine ↔
//! shell relationship is one binary's two halves.
//!
//! Phase 1 of GitHub issue #212 (Dispatcher extraction). The
//! struct lives next to `App` (its sole consumer); the previous
//! core/front directory experiment (Phase 4) was reverted in
//! favour of a flat layout — see `src/lib.rs` header.

use std::time::Duration;

use crossterm::event::KeyEvent;
use log::debug;

use super::overlay::OverlayThrottle;
use super::sidebar::SidebarPolicy;
use crate::UserCommand;
use crate::compositor::op::Op;
use crate::compositor::{Compositor, Context};
use crate::lua::LuaHandle;
use crate::map::MapAction;
use crate::map::MapHandle;
use crate::map::render::frame::MapFrame;
use crate::theme::ThemeId;
use crate::theme::UiTheme;

pub(super) struct Dispatcher {
    pub(super) map: MapHandle,
    pub(super) running: bool,
    pub(super) theme_id: ThemeId,
    pub(super) ui_theme: UiTheme,
    pub(super) lua: LuaHandle,
    pub(super) compositor: Compositor,
    pub(super) sidebar: SidebarPolicy,
    pub(super) cursor: Option<(u16, u16)>,
    pub(super) overlay: OverlayThrottle,
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
        // `Map(Redraw)` reads no frame; pass None.
        self.dispatch(UserCommand::Map(MapAction::Redraw), None);
    }

    /// Build the [`Context`] snapshot read by component hooks.
    pub(super) fn context(&self) -> Context {
        Context {
            theme_id: self.theme_id,
            cursor: self.cursor,
        }
    }

    /// Forward a frame-ready signal to Lua subscribers. Called from
    /// `App::handle_event` when a fresh `MapFrame` arrives off the
    /// render thread.
    pub(super) fn notify_frame_ready(&self) {
        self.lua.notify_frame_ready();
    }

    /// Drain queued Lua-side ops and apply them. App calls this once
    /// per loop iteration after event handling, before render.
    pub(super) fn apply_lua_ops(&mut self, frame: Option<&MapFrame>) {
        let ops = self.lua.drain_ops();
        self.apply_ops(ops, frame);
    }

    /// Apply a batch of [`Op`]s from any source (Lua callbacks,
    /// component handlers, component polls). All converge on this
    /// single applier.
    pub(super) fn apply_ops(&mut self, ops: Vec<Op>, frame: Option<&MapFrame>) {
        for op in ops {
            match op {
                Op::Push { id, component } => {
                    self.compositor.push_with_id(id, component);
                }
                Op::Close(id) => self.compositor.close_by_id(id),
                Op::Command(cmd) => self.dispatch(cmd, frame),
            }
        }
    }

    /// Deliver a key event to the compositor and apply any resulting
    /// ops. Called from `App::handle_input` for non-Ctrl-C key
    /// events.
    pub(super) fn handle_key_event(&mut self, key: KeyEvent, frame: Option<&MapFrame>) {
        let ctx = self.context();
        let ops = self.compositor.handle_event(key, &ctx);
        self.apply_ops(ops, frame);
    }

    /// Drive a single `compositor.poll` pass plus the sidebar
    /// auto-open observation. Called once per loop iteration.
    pub(super) fn poll_compositor(&mut self, frame: Option<&MapFrame>) {
        let ctx = self.context();
        let ops = self.compositor.poll(&ctx);
        self.apply_ops(ops, frame);

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

    /// Execute a [`UserCommand`] and broadcast the post-effect
    /// notification on the Lua event bus. Single side-effect
    /// boundary for app-level state changes.
    ///
    /// `frame` carries the latest `MapFrame` (held by `App`) for the
    /// `ExportFrame` arm. Other arms ignore it. Pass `None` if no
    /// frame has arrived yet (`App::run`'s initial redraw etc.).
    pub(super) fn dispatch(&mut self, msg: UserCommand, frame: Option<&MapFrame>) {
        let snapshot = msg.clone();
        match msg {
            UserCommand::Map(action) => {
                if self.map.apply_action(&action) {
                    self.request_map_redraw();
                }
            }
            UserCommand::Quit => {
                debug!("UserCommand::Quit — stopping event loop");
                self.running = false;
            }
            UserCommand::SetTheme(new_id) => self.switch_theme(new_id),
            UserCommand::CursorMoved(col, row) => {
                self.cursor = Some((col, row));
            }
            UserCommand::CycleFocus(forward) => {
                self.compositor.cycle(forward);
            }
            UserCommand::Resize(cols, rows) => self.handle_resize(cols, rows),
            UserCommand::ExportFrame => self.export_current_frame(frame),
            UserCommand::ToggleSidebar => self.toggle_sidebar(),
        }
        self.notify_post_command(&snapshot);
    }

    /// Broadcast post-effect notifications for the variants that
    /// plugins want to observe. Skips noisy / internal commands so
    /// the bus surface stays meaningful — bus events are "something
    /// observable happened to the app", not "every state mutation".
    fn notify_post_command(&self, msg: &UserCommand) {
        match msg {
            UserCommand::Map(MapAction::Jump(ll)) => self.lua.notify_map_jumped(*ll),
            UserCommand::Map(MapAction::SetZoom(z)) => self.lua.notify_map_zoom_set(*z),
            UserCommand::Map(MapAction::FlyTo { center, zoom }) => {
                self.lua.notify_map_flew_to(*center, *zoom);
            }
            UserCommand::SetTheme(new_id) => self.lua.notify_theme_changed(new_id.name()),
            UserCommand::Resize(cols, rows) => self.lua.notify_resized(*cols, *rows),
            UserCommand::ExportFrame => self.lua.notify_frame_exported(),
            // Noisy or internal — `PanCells`, `ZoomAt`, `CursorMoved`,
            // `CycleFocus`, the discrete `Pan*` keymap actions, and
            // `Quit` (the App is tearing down anyway) are deliberately
            // not broadcast. Adding them later is one match arm.
            _ => {}
        }
    }

    fn switch_theme(&mut self, new_id: ThemeId) {
        self.theme_id = new_id;
        self.ui_theme = UiTheme::from_palette(new_id.palette());
        self.map.set_theme(new_id);
        self.request_map_redraw();
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        let map_cols = self.sidebar.effective_map_cols(cols);
        self.map.handle_resize(map_cols, rows);
        self.request_map_redraw();
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar.toggle();
        // The visible map area shrinks/expands — re-run the resize
        // path so the render thread allocates the right canvas size.
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        self.handle_resize(cols, rows);
    }

    fn export_current_frame(&self, frame: Option<&MapFrame>) {
        let Some(frame) = frame else {
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
