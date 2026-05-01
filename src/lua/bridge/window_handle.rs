//! `ttymap.api.window.open(spec) -> WindowHandle` — push a focused
//! component onto the compositor stack, return a Lua-facing handle
//! whose only method is `close()` (idempotent).
//!
//! The handle holds a shared atomic flag; `close()` flips it. The
//! `LuaWindowComponent` it pushed checks the flag on its next poll
//! tick and pops itself off the stack via `win.close()`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mlua::UserData;

/// Shared close flag between the Lua-side handle and the Rust-side
/// component. Cloned at construction; either side flipping it
/// triggers the next poll-tick `win.close()`.
#[derive(Clone, Default)]
pub struct CloseFlag(Arc<AtomicBool>);

impl CloseFlag {
    // Relaxed is sufficient: the flag is the only shared state and
    // there is no companion data whose write must be ordered with
    // respect to the flip.
    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// Lua-facing handle returned by `ttymap.api.window.open(...)`.
/// Idempotent `:close()` — flipping a flipped flag is a no-op.
pub struct WindowHandle {
    flag: CloseFlag,
}

impl WindowHandle {
    /// Build a handle that signals close via the shared `flag`.
    pub fn new(flag: CloseFlag) -> Self {
        Self { flag }
    }
}

impl UserData for WindowHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("close", |_, this, _: ()| {
            this.flag.request();
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_flips_flag_idempotent() {
        let flag = CloseFlag::default();
        let lua = mlua::Lua::new();
        let ud = lua
            .create_userdata(WindowHandle::new(flag.clone()))
            .unwrap();
        lua.load("local h = ...; h:close(); h:close()")
            .call::<()>(ud)
            .unwrap();
        assert!(flag.take());
        assert!(!flag.take());
    }
}
