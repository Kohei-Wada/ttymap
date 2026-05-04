//! `ttymap.map` userdata — the map-action surface every plugin uses
//! to recentre / zoom / fly-to / read the current centre.
//!
//! All mutators are fire-and-forget: each call enqueues an
//! [`Op::Command(UserCommand::Map(...))`] onto the shared
//! [`OpsBuffer`](crate::compositor::op::OpsBuffer) and the App drains
//! it once per iteration. Read methods (`center`, no-arg `zoom`)
//! consult shared `Arc<Mutex<...>>` cells the host refreshes on every
//! dispatch path that carries a `Window` / `MapApi`, so callers see
//! the latest values from any callback context.

use std::sync::{Arc, Mutex};

use mlua::UserData;

use crate::UserCommand;
use crate::compositor::op::{Op, OpsBuffer};
use crate::geo::LonLat;
use crate::map::MapAction;

pub(super) struct HostMap {
    /// Shared op buffer the lua subsystem drains every iteration.
    /// Fire-and-forget Lua intents (`jump` / `zoom` / `fly_to`)
    /// enqueue an `Op::Command(UserCommand::Map(...))`; the host treats
    /// them identically to a keymap-driven dispatch.
    ops: OpsBuffer,
    center: Arc<Mutex<LonLat>>,
    zoom: Arc<Mutex<f64>>,
}

impl HostMap {
    pub(super) fn new(ops: OpsBuffer, center: Arc<Mutex<LonLat>>, zoom: Arc<Mutex<f64>>) -> Self {
        Self { ops, center, zoom }
    }
}

impl UserData for HostMap {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.map:jump(lon, lat)` — request the map recentre on
        // the given coordinate. Enqueues `UserCommand::Map(Jump)` onto
        // the shared op buffer so the host treats it identically to a
        // keymap-driven jump.
        methods.add_method("jump", |_, this, (lon, lat): (f64, f64)| {
            this.ops
                .borrow_mut()
                .push(Op::Command(UserCommand::Map(MapAction::Jump(LonLat {
                    lon,
                    lat,
                }))));
            Ok(())
        });

        // `ttymap.map:zoom([level])` — overloaded:
        //   `:zoom(level)` queues a zoom request (clamped host-side
        //   in `MapState::process_action`). Fire-and-forget.
        //   `:zoom()` (no arg) returns the current zoom level read
        //   from the shared `Arc<Mutex<f64>>` the host refreshes on
        //   the same dispatch paths it refreshes `:center()` on.
        // mlua dispatches by the supplied argument signature: nil →
        // `None` (getter), number → `Some(level)` (setter).
        methods.add_method("zoom", |_, this, level: Option<f64>| match level {
            Some(z) => {
                this.ops
                    .borrow_mut()
                    .push(Op::Command(UserCommand::Map(MapAction::SetZoom(z))));
                Ok(mlua::Value::Nil)
            }
            None => {
                let z = *this.zoom.lock().expect("zoom mutex poisoned");
                Ok(mlua::Value::Number(z))
            }
        });

        // `ttymap.map:fly_to(lon, lat, zoom)` — composite recenter +
        // zoom in one dispatch. Emitting `jump` + `zoom` separately
        // would render two frames; this routes through `MapFlyTo`
        // so the user sees a single transition.
        methods.add_method("fly_to", |_, this, (lon, lat, zoom): (f64, f64, f64)| {
            this.ops
                .borrow_mut()
                .push(Op::Command(UserCommand::Map(MapAction::FlyTo {
                    center: LonLat { lon, lat },
                    zoom,
                })));
            Ok(())
        });

        // `ttymap.map:center()` -> lon, lat — current map centre, kept
        // fresh by the host before each dispatch path that carries a
        // `Window` / `MapApi`. Plugins use this to scope upstream
        // queries (e.g. an OpenSky bounding box around the user's
        // view).
        methods.add_method("center", |_, this, _: ()| {
            let ll = *this.center.lock().expect("center mutex poisoned");
            Ok((ll.lon, ll.lat))
        });
    }
}
