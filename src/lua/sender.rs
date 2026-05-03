//! Boundary type that hides the `AppEvent` channel from the rest of
//! the Lua subsystem.
//!
//! Constructed at the composition root from a `Sender<AppEvent>`
//! clone, threaded through the lua plumbing chain (build_registrar
//! → register_builtin_plugins → register_one → fresh_load → install)
//! and finally handed to [`super::api`] host userdatas. Those
//! userdatas only know how to emit a [`crate::frontend::UserIntent`];
//! the wrap into `AppEvent::Intent` happens here, so no other file
//! in `src/lua/` imports the app-level event type.
//!
//! Plugin trust model is nvim-style: plugins are user-installed and
//! trusted, so any `UserIntent` variant is fair game (a plugin can
//! quit the app, switch the theme, toggle the sidebar, etc.). The
//! sender only enforces the channel-vs-event-type boundary — it
//! does *not* enforce a permission allow-list.

use std::sync::mpsc;

use crate::frontend::{AppEvent, UserIntent};

/// Cheap-clone Sender wrapper. The lua module passes this around
/// instead of `mpsc::Sender<AppEvent>`; the wrap into
/// `AppEvent::Intent` is contained here.
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

    /// Send a host intent. Wraps as [`AppEvent::Intent`] and pushes
    /// onto the App's unified queue. Send errors (channel closed at
    /// app teardown) are silently ignored — the host is going away.
    pub fn emit(&self, intent: UserIntent) {
        let _ = self.inner.send(AppEvent::Intent(intent));
    }
}
