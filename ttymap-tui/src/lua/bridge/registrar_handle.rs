//! `ttymap.register_palette_command(spec) -> PaletteCommandHandle`
//! and `ttymap.register_keybind(key, fn) -> KeybindHandle` —
//! Lua-facing handles for the two registrar-backed registration
//! surfaces.
//!
//! Both handles expose a single `:remove()` method that is **stubbed
//! today**: it logs a warning and returns. The API shape (handle
//! return + `:remove()` verb) is unified with [`super::event_handle::EventHandle`]
//! so plugin authors can write a single `ttymap.plugin` Lua wrapper
//! that disposes everything uniformly. The Rust-side wiring needs a
//! separate refactor (a live `Rc<RefCell<PluginRegistry>>` that
//! `BaseLayer` and the palette installer query each frame, instead
//! of moving `Vec`s out of the registrar at build time) — that lands
//! in a follow-up PR. The stub keeps the public surface stable so
//! the Lua wrapper is forward-compatible.
//!
//! Each handle carries an `id: u64` allocated from the slot's
//! per-load counter at the registration call site. The ID is opaque
//! today (no consumer matches against it) but is the natural
//! identity the future live-registry refactor will key on.

use std::sync::atomic::{AtomicU64, Ordering};

use mlua::UserData;

/// Process-global counter for registrar handle IDs. Bumped once per
/// `register_palette_command` or `register_keybind` call. The IDs
/// are opaque today (the `:remove()` stubs don't match against
/// anything) but are unique-per-process so the future
/// live-registry refactor can key on them without aliasing across
/// plugins or restarts within the same run.
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(0);

/// Allocate the next registrar handle ID. Used by the
/// `register_palette_command` and `register_keybind` capturers in
/// [`crate::lua::api::register`].
pub fn allocate_handle_id() -> u64 {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Lua-facing handle returned by `ttymap.register_palette_command(...)`.
///
/// `:remove()` is a stub that logs a warning. Live removal needs a
/// runtime-mutable palette command list, which is a separate
/// refactor.
pub struct PaletteCommandHandle {
    id: u64,
}

impl PaletteCommandHandle {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

impl UserData for PaletteCommandHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            log::warn!(
                "ttymap.register_palette_command: handle {}: :remove() not yet wired \
                 (palette is baked into the BaseLayer at startup; reload the host to drop)",
                this.id
            );
            Ok(())
        });
    }
}

/// Lua-facing handle returned by `ttymap.register_keybind(...)`.
///
/// `:remove()` is a stub that logs a warning. Live removal needs a
/// runtime-mutable activation table on `BaseLayer`; landing that
/// touches the dispatcher's hot path so it lives in a follow-up.
pub struct KeybindHandle {
    id: u64,
}

impl KeybindHandle {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

impl UserData for KeybindHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            log::warn!(
                "ttymap.register_keybind: handle {}: :remove() not yet wired \
                 (keybinds are baked into the BaseLayer at startup; reload the host to drop)",
                this.id
            );
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_command_handle_remove_is_idempotent_no_op() {
        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(PaletteCommandHandle::new(7)).unwrap();
        // Two calls in a row must not error.
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .expect("remove no-op must not error");
    }

    #[test]
    fn keybind_handle_remove_is_idempotent_no_op() {
        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(KeybindHandle::new(13)).unwrap();
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .expect("remove no-op must not error");
    }
}
