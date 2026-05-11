//! Runtime handle to the Lua subsystem.
//!
//! Cheap-to-clone Rc/Arc bundle that App + Dispatcher both hold so
//! each can reach the Lua-side state they need (drain ops, sync
//! per-plugin view mirrors, tick on draw, mirror the latest frame,
//! set the tile attribution string) without routing through the
//! other half.
//!
//! Event publishing is **not** part of this surface. The [`EventBus`]
//! lives next to LuaHandle in [`crate::lua::LuaSubsystem`] and is
//! held directly by App / Dispatcher — publishes happen inline
//! alongside the state mutation that produced them, not through a
//! `notify_*` wrapper layer here. See #334.

use std::rc::Rc;
use std::sync::Arc;

use crate::compositor::op::{Op, OpsBuffer};
use crate::event::EventBus;
use crate::lua::MapApi;
use crate::lua::host::{LuaHostHandles, LuaHostShared};
use crate::lua::tick;
use ttymap_engine::geo::LonLat;
use ttymap_engine::map::render::frame::MapFrame;

/// Runtime-held part of the Lua subsystem (built by
/// [`crate::lua::build_subsystem`]). Clone is free — every field is
/// already an `Rc` / `Arc` / cheap newtype, so App and Dispatcher
/// share one logical instance through their clones.
///
/// The bus is held here only because [`Self::tick`] needs it to
/// fan out the per-frame `"tick"` event. App / Dispatcher reach the
/// bus directly via the clone stored on [`crate::lua::LuaSubsystem`].
#[derive(Clone)]
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

    /// Mirror the latest [`MapFrame`] into the shared cell that
    /// `ttymap.api.frame.to_ansi()` reads from. App calls this on
    /// every `AppEvent::FrameReady` before publishing `Event::FrameReady`
    /// so subscribers that immediately query `to_ansi()` see the
    /// fresh frame.
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
