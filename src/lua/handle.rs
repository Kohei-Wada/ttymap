//! Runtime handle to the Lua subsystem.
//!
//! App stores one of these as a private field. It exposes
//! **semantic** methods (`notify_*`, `tick`, `sync_view`,
//! `drain_ops`) so App never names the [`LuaEventBus`], the
//! event-name constants, or the per-plugin host channels. The
//! "App doesn't know how Lua is wired internally" boundary —
//! all bus dispatch and host-state plumbing lives behind this type.

use crate::compositor::MapApi;
use crate::geo::LonLat;
use crate::lua::api::LuaHostHandles;
use crate::lua::op::{Op, OpsBuffer};
use crate::lua::registry::{LuaEventBus, names};

/// Runtime-held part of the Lua subsystem (built by
/// [`crate::lua::build_subsystem`] and lifted out of the registrar by
/// [`Self::take_from_registrar`]). Wraps the event bus and per-plugin
/// host-state channels so callers (App) interact through
/// semantic methods rather than touching the bus or channels
/// directly.
pub struct LuaHandle {
    bus: LuaEventBus,
    host_handles: Vec<LuaHostHandles>,
    /// Shared buffer that Lua callbacks push [`Op`]s into; App
    /// drains via [`Self::drain_ops`] once per loop iteration.
    ops: OpsBuffer,
}

impl LuaHandle {
    pub fn new(bus: LuaEventBus, host_handles: Vec<LuaHostHandles>, ops: OpsBuffer) -> Self {
        Self {
            bus,
            host_handles,
            ops,
        }
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
        self.bus.dispatch_tick(map);
    }

    /// Notify Lua observers that a fresh frame was drained from the
    /// render thread.
    pub fn notify_frame_ready(&self) {
        self.bus.dispatch(names::FRAME_READY, ());
    }

    /// Notify Lua observers that the map centred on a new location.
    pub fn notify_map_jumped(&self, ll: LonLat) {
        self.bus.dispatch(names::MAP_JUMPED, (ll.lon, ll.lat));
    }

    /// Notify Lua observers that the zoom level was set explicitly
    /// (via `:zoom` or `MapAction::SetZoom`).
    pub fn notify_map_zoom_set(&self, zoom: f64) {
        self.bus.dispatch(names::MAP_ZOOM_SET, zoom);
    }

    /// Notify Lua observers that the map flew to a (centre, zoom)
    /// pair in one combined dispatch.
    pub fn notify_map_flew_to(&self, center: LonLat, zoom: f64) {
        self.bus
            .dispatch(names::MAP_FLEW_TO, (center.lon, center.lat, zoom));
    }

    /// Notify Lua observers that the active theme switched.
    pub fn notify_theme_changed(&self, theme_name: &str) {
        self.bus.dispatch(names::THEME_CHANGED, theme_name);
    }

    /// Notify Lua observers that the terminal resized.
    pub fn notify_resized(&self, cols: u16, rows: u16) {
        self.bus.dispatch(names::RESIZED, (cols, rows));
    }

    /// Notify Lua observers that an export wrote a frame to disk.
    pub fn notify_frame_exported(&self) {
        self.bus.dispatch(names::FRAME_EXPORTED, ());
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
