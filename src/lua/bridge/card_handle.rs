//! `ttymap.api.card.open(spec) -> CardHandle` — push a focused
//! component onto the compositor stack, return a Lua-facing handle
//! whose only method is `close()` (idempotent).
//!
//! The handle holds a reserved [`CardId`] and a clone of the
//! Lua subsystem's shared [`OpsBuffer`]. `close()` enqueues an
//! [`Op::Close`] keyed by the id; the App drains the buffer per
//! iteration and pops the matching component via
//! [`crate::compositor::Compositor::close_by_id`].
//!
//! Replaces the older `Arc<AtomicBool>` flag + per-component
//! `Component::poll` polling pattern: there's no per-frame
//! "is this card asking to close?" scan; the close is a single
//! push to the buffer, applied directly by id.

use mlua::UserData;

use crate::compositor::CardId;
use crate::lua::op::{Op, OpsBuffer};

/// Lua-facing handle returned by `ttymap.api.card.open(...)`.
/// Idempotent `:close()` — pushing two `Op::Close(id)` for the same
/// id is harmless (the second `close_by_id` call is a no-op once the
/// component is already off the stack).
pub struct CardHandle {
    id: CardId,
    ops: OpsBuffer,
}

impl CardHandle {
    /// Build a handle that requests close via [`Op::Close`] keyed by
    /// the supplied `id`.
    pub fn new(id: CardId, ops: OpsBuffer) -> Self {
        Self { id, ops }
    }
}

impl UserData for CardHandle {
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
            .create_userdata(CardHandle::new(id, ops.clone()))
            .unwrap();
        lua.load("local h = ...; h:close(); h:close()")
            .call::<()>(ud)
            .unwrap();
        let drained: Vec<Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 2, "two close() calls -> two ops queued");
        for op in drained {
            match op {
                Op::Close(got) => assert_eq!(got, id),
                other => panic!("expected Op::Close, got {:?}", other),
            }
        }
    }
}
