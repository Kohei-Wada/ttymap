//! Runtime handle to the Lua subsystem.
//!
//! App stores one of these as a private field. It exposes
//! **semantic** methods (`notify_*`, `tick`, `sync_view`,
//! `drain_ops`) so App never names the [`EventBus`], the event
//! variants, or the per-plugin host channels. The "App doesn't know
//! how Lua is wired internally" boundary — all bus dispatch and
//! host-state plumbing lives behind this type.

use std::rc::Rc;
use std::sync::Arc;

use crate::compositor::op::{Op, OpsBuffer};
use crate::event::{Event, EventBus, Level};
use crate::lua::MapApi;
use crate::lua::host::{LuaHostHandles, LuaHostShared};
use crate::lua::tick;
use ttymap_engine::geo::LonLat;
use ttymap_engine::map::render::frame::MapFrame;

/// Runtime-held part of the Lua subsystem (built by
/// [`crate::lua::build_subsystem`]). Wraps the event bus and
/// per-plugin host-state channels so callers (App) interact through
/// semantic methods rather than touching the bus or channels
/// directly.
///
/// The bus is wrapped in [`Rc`] because the same instance is
/// referenced by every `EventHandle` userdata returned to Lua
/// (`ttymap.on_event` / `ttymap.api.frame.on_tick`) — they keep a
/// `Rc<EventBus>` to call `remove(...)` on `:remove()`.
pub struct LuaHandle {
    bus: Rc<EventBus>,
    host_handles: Vec<LuaHostHandles>,
    /// Shared buffer that Lua callbacks push [`Op`]s into; App
    /// drains via [`Self::drain_ops`] once per loop iteration.
    ops: OpsBuffer,
    /// Read-mostly snapshot the Lua bridge exposes via `ttymap.*`
    /// userdatas. The handle keeps a clone so App can write into
    /// shared cells (e.g. `current_frame`) through semantic methods
    /// like [`Self::set_current_frame`] without naming
    /// `LuaHostShared` directly.
    shared: Arc<LuaHostShared>,
}

impl LuaHandle {
    pub fn new(
        bus: Rc<EventBus>,
        host_handles: Vec<LuaHostHandles>,
        ops: OpsBuffer,
        shared: Arc<LuaHostShared>,
    ) -> Self {
        Self {
            bus,
            host_handles,
            ops,
            shared,
        }
    }

    /// Borrow the underlying [`EventBus`] for direct publishes (e.g.
    /// the App-mpsc drain branch that handles
    /// [`crate::app::AppEvent::Bus`] hands the event straight to
    /// `bus.publish`). Most callers should prefer the `notify_*`
    /// semantic methods below.
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Take every [`Op`] enqueued by Lua callbacks since the last
    /// drain. App calls this once per loop iteration and applies
    /// each op to the compositor / dispatch path.
    pub fn drain_ops(&self) -> Vec<Op> {
        std::mem::take(&mut *self.ops.borrow_mut())
    }

    /// Fire the per-frame `"tick"` event. Called by `ui::draw` once
    /// per frame against the live [`MapApi`].
    pub fn tick(&self, map: &mut MapApi<'_>) {
        tick::dispatch_tick(&self.bus, map);
    }

    /// Notify observers that a fresh frame was drained from the
    /// render thread.
    pub fn notify_frame_ready(&self) {
        self.bus.publish(Event::FrameReady);
    }

    /// Mirror the latest [`MapFrame`] into the shared cell that
    /// `ttymap.api.frame.to_ansi()` reads from. Called by App on
    /// every `AppEvent::FrameReady` before [`Self::notify_frame_ready`]
    /// so subscribers that immediately query `to_ansi()` see the
    /// fresh frame.
    pub fn set_current_frame(&self, frame: MapFrame) {
        if let Ok(mut slot) = self.shared.current_frame.lock() {
            *slot = Some(frame);
        }
    }

    /// Notify observers that the map centred on a new location.
    pub fn notify_map_jumped(&self, ll: LonLat) {
        self.bus.publish(Event::MapJumped(ll));
    }

    /// Notify observers that the zoom level was set explicitly
    /// (via `:zoom` or `MapAction::SetZoom`).
    pub fn notify_map_zoom_set(&self, zoom: f64) {
        self.bus.publish(Event::MapZoomSet(zoom));
    }

    /// Notify observers that the map flew to a (centre, zoom) pair
    /// in one combined dispatch.
    pub fn notify_map_flew_to(&self, center: LonLat, zoom: f64) {
        self.bus.publish(Event::MapFlewTo(center, zoom));
    }

    /// Notify observers that the active theme switched.
    pub fn notify_theme_changed(&self, theme_name: &str) {
        self.bus
            .publish(Event::ThemeChanged(theme_name.to_string()));
    }

    /// Notify observers that the terminal resized.
    pub fn notify_resized(&self, cols: u16, rows: u16) {
        self.bus.publish(Event::Resized(cols, rows));
    }

    /// Push a transient toast onto the bus. The bundled `notify.lua`
    /// renderer subscribes and paints recent ones top-left for ~3s.
    /// Callable from any main-thread Rust path; cross-thread
    /// producers route through `AppEvent::Bus` instead.
    pub fn notify(&self, message: impl Into<String>, level: Level) {
        self.bus.publish(Event::Notify {
            message: message.into(),
            level,
        });
    }

    /// Refresh the per-plugin `center` / `zoom` mirror cells that
    /// `ttymap.map:center()` / `:zoom()` Lua accessors read from.
    /// Called once per tick from App's housekeeping pass.
    pub fn sync_view(&self, center: LonLat, zoom: f64) {
        for handles in &self.host_handles {
            if let Ok(mut cell) = handles.center.lock() {
                *cell = center;
            }
            if let Ok(mut cell) = handles.zoom.lock() {
                *cell = zoom;
            }
        }
    }
}
