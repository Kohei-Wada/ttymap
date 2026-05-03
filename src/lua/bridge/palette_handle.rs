//! `ttymap.api.palette.open(spec) -> PaletteHandle` — push a palette
//! provider component onto the compositor stack and return a
//! Lua-facing handle whose only method is `close()` (idempotent).
//!
//! Structurally identical to [`super::card_handle::CardHandle`]:
//! the handle holds a shared atomic flag, `close()` flips it, and the
//! wrapped [`crate::frontend::palette::PaletteComponent`] checks the flag on its
//! next poll tick (via [`super::card_handle::CloseFlagWrapper`]) and
//! pops itself off the stack via `win.close()`. Kept as its own type
//! so callers know which kind of primitive they have — the Rust-side
//! wiring is shared, but the Lua-side identity is not.

use mlua::UserData;

use super::card_handle::CloseFlag;

/// Lua-facing handle returned by `ttymap.api.palette.open(...)`.
/// Idempotent `:close()` — flipping a flipped flag is a no-op.
pub struct PaletteHandle {
    flag: CloseFlag,
}

impl PaletteHandle {
    /// Build a handle that signals close via the shared `flag`.
    pub fn new(flag: CloseFlag) -> Self {
        Self { flag }
    }
}

impl UserData for PaletteHandle {
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
            .create_userdata(PaletteHandle::new(flag.clone()))
            .unwrap();
        lua.load("local h = ...; h:close(); h:close()")
            .call::<()>(ud)
            .unwrap();
        assert!(flag.take());
        assert!(!flag.take());
    }
}
