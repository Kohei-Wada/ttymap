//! Runtime handle to the Lua subsystem.
//!
//! Bundle of Rc/Arc fields Dispatcher holds so it can reach the
//! Lua-side state it needs (drain ops, sync per-plugin view
//! mirrors, tick on draw, mirror the latest frame, set the tile
//! attribution string).
//!
//! Event publishing is not part of this surface. The [`EventBus`]
//! lives next to LuaHandle in [`crate::LuaSubsystem`] and is
//! held directly by App; Dispatcher accumulates events into a
//! buffer App drains and publishes (#334).
//!
//! [`EventBus`]: ttymap_core::event::EventBus

use std::rc::Rc;
use std::sync::Arc;

use crate::MapApi;
use crate::host::{LuaHostHandles, LuaHostShared};
use crate::tick::TickRegistry;
use ttymap_engine::geo::LonLat;
use ttymap_engine::map::render::frame::MapFrame;
use ttymap_tui::compositor::op::{Op, OpsBuffer};

/// Runtime-held part of the Lua subsystem (built by
/// [`crate::build_subsystem`]).
///
/// Holds the per-frame [`TickRegistry`] (so [`Self::tick`] can fan
/// out the `tick` hook) plus the read-mostly snapshot
/// [`LuaHostShared`] that the bridge namespaces expose. App reaches
/// the typed-event bus directly via the clone stored on
/// [`crate::LuaSubsystem`].
pub struct LuaHandle {
    ticks: Rc<TickRegistry>,
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
        ticks: Rc<TickRegistry>,
        host_handles: Vec<LuaHostHandles>,
        ops: OpsBuffer,
        shared: Arc<LuaHostShared>,
    ) -> Self {
        Self {
            ticks,
            host_handles,
            ops,
            shared,
        }
    }

    /// Take every [`Op`] enqueued by Lua callbacks since the last
    /// drain. App calls this once per loop iteration and applies
    /// each op to the compositor / dispatch path.
    pub fn drain_ops(&self) -> Vec<Op> {
        std::mem::take(&mut *self.ops.borrow_mut())
    }

    /// Fire the per-frame `tick` hook against a live [`MapApi`].
    /// Called by `ui::draw` once per frame after composing the map.
    pub fn tick(&self, map: &mut MapApi<'_>) {
        self.ticks.dispatch(map);
    }

    /// Mirror the latest [`MapFrame`] into the shared cell that
    /// `ttymap.api.frame.to_ansi()` reads from. App calls this on
    /// every `AppEvent::FrameReady` so Lua callers querying
    /// `to_ansi()` see the fresh frame.
    pub fn set_current_frame(&self, frame: MapFrame) {
        if let Ok(mut slot) = self.shared.current_frame.lock() {
            *slot = Some(frame);
        }
    }

    /// Set the tile provider's attribution string the
    /// `ttymap.tile:attribution()` Lua surface returns. Called by
    /// the binary once at startup, after the tile cache spins up
    /// (the engine config — which determines the active backend's
    /// attribution — comes from the Lua bootstrap, so the cell is
    /// populated post-`build_subsystem`).
    pub fn set_attribution(&self, attribution: Option<String>) {
        self.shared.set_attribution(attribution);
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
