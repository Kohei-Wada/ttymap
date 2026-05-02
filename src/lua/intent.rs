//! Lua-side intent vocabulary.
//!
//! [`LuaIntent`] is what fires when a Lua plugin calls a host method
//! that wants to mutate app state — `ttymap.map:jump`, `:zoom`,
//! `:fly_to`, `ttymap.api.frame.export`. The Lua subsystem doesn't
//! know about [`crate::frontend::AppMsg`] / [`crate::map::Action`];
//! it only emits its own intent variants. The frontend layer
//! translates them on the way through `handle_event`, so the Lua
//! module stays bounded to its own vocabulary.
//!
//! Why a separate enum (rather than reusing `AppMsg` directly):
//! the lua module is otherwise a peer subsystem to render / input /
//! frame timer — it should mediate events on the bus, not import
//! the app's imperative vocabulary. Modelling Lua-originated intents
//! as their own type keeps the boundary visible.

use crate::geo::LonLat;

/// One Lua-originated intent, fired through [`super::sender::LuaSender`]
/// from `ttymap.*` host bindings. The frontend translates each
/// variant to the matching [`crate::frontend::AppMsg`] inside its
/// `handle_event` arm — the lua module never spells those types.
#[derive(Debug, Clone, PartialEq)]
pub enum LuaIntent {
    /// `ttymap.map:jump(lon, lat)` — recentre the map.
    MapJump(LonLat),
    /// `ttymap.map:zoom(level)` — set zoom directly (clamped
    /// host-side).
    MapZoomSet(f64),
    /// `ttymap.map:fly_to(lon, lat, zoom)` — composite recenter +
    /// zoom in one dispatch (single render at the new view).
    MapFlyTo { center: LonLat, zoom: f64 },
    /// `ttymap.api.frame.export()` — snapshot the current frame to
    /// disk.
    FrameExport,
}
