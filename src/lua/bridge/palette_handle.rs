//! `ttymap.api.palette.open(spec) -> PaletteHandle` — push a palette
//! provider component onto the compositor stack and return a
//! Lua-facing handle whose only method is `close()` (idempotent).
//!
//! Structurally identical to [`super::card_handle::CardHandle`]:
//! the handle holds a reserved [`CardId`] and a clone of the Lua
//! subsystem's shared [`OpsBuffer`]; `close()` enqueues an
//! [`Op::Close`] that the App applies via
//! [`crate::frontend::compositor::Compositor::close_by_id`]. Kept as
//! its own type so callers know which kind of primitive they have —
//! the Rust-side wiring is shared, but the Lua-side identity is not.

use mlua::UserData;

use crate::frontend::compositor::CardId;
use crate::lua::op::{Op, OpsBuffer};

/// Lua-facing handle returned by `ttymap.api.palette.open(...)`.
/// Idempotent `:close()` — pushing two `Op::Close(id)` for the same
/// id is harmless (the second `close_by_id` call is a no-op once the
/// component is already off the stack).
pub struct PaletteHandle {
    id: CardId,
    ops: OpsBuffer,
}

impl PaletteHandle {
    /// Build a handle that requests close via [`Op::Close`] keyed by
    /// the supplied `id`.
    pub fn new(id: CardId, ops: OpsBuffer) -> Self {
        Self { id, ops }
    }
}

impl UserData for PaletteHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("close", |_, this, _: ()| {
            this.ops.borrow_mut().push(Op::Close(this.id));
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::op::new_ops_buffer;

    #[test]
    fn close_enqueues_op_close_idempotent() {
        let ops = new_ops_buffer();
        let id = CardId::next();
        let lua = mlua::Lua::new();
        let ud = lua
            .create_userdata(PaletteHandle::new(id, ops.clone()))
            .unwrap();
        lua.load("local h = ...; h:close(); h:close()")
            .call::<()>(ud)
            .unwrap();
        let drained: Vec<Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 2);
        for op in drained {
            match op {
                Op::Close(got) => assert_eq!(got, id),
                other => panic!("expected Op::Close, got {:?}", other),
            }
        }
    }
}
