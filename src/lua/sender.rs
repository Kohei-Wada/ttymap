//! Boundary type that hides the `AppEvent` channel from the rest of
//! the Lua subsystem.
//!
//! Constructed at the composition root from a `Sender<AppEvent>`
//! clone, threaded through the lua plumbing chain (build_registrar
//! → register_builtin_plugins → register_one → fresh_load → install)
//! and finally handed to [`super::ttymap`] host userdatas. Those
//! userdatas only know how to emit a [`super::intent::LuaIntent`];
//! the wrap into `AppEvent::LuaIntent` happens here, so no other
//! file in `src/lua/` imports the app-level event type.

use std::sync::mpsc;

use crate::frontend::AppEvent;

use super::intent::LuaIntent;

/// Cheap-clone Sender wrapper. The lua module passes this around
/// instead of `mpsc::Sender<AppEvent>`; the wrap into
/// `AppEvent::LuaIntent` is contained here.
#[derive(Clone)]
pub struct LuaSender {
    inner: mpsc::Sender<AppEvent>,
}

impl LuaSender {
    /// Construct from the App-level event channel. Called once at
    /// the boundary (`Frontend::new`); the lua module receives the
    /// resulting `LuaSender` through the registration chain.
    pub fn new(inner: mpsc::Sender<AppEvent>) -> Self {
        Self { inner }
    }

    /// Send a Lua intent. Wraps as [`AppEvent::LuaIntent`] and pushes
    /// onto the App's unified queue. Send errors (channel closed at
    /// app teardown) are silently ignored — the host is going away.
    pub fn emit(&self, intent: LuaIntent) {
        let _ = self.inner.send(AppEvent::LuaIntent(intent));
    }
}
