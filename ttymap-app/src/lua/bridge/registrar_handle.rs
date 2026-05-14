//! `ttymap.register_palette_command(spec) -> PaletteCommandHandle`
//! and `ttymap.register_keybind(key, fn) -> KeybindHandle` —
//! Lua-facing handles for the two registrar-backed registration
//! surfaces.
//!
//! Both handles expose a single `:remove()` method that drops the
//! matching entry from the live
//! [`LuaRegistry`](crate::lua::registrar::LuaRegistry). The
//! verb matches [`super::event_handle::EventHandle::remove`] so a
//! plugin author can dispose any registration uniformly. Idempotent
//! — `:remove()` on an already-removed handle is a no-op.
//!
//! Each handle carries an `id: u64` allocated at registration call
//! site (the same id stored alongside the entry in the registry),
//! plus a clone of the `Rc<RefCell<LuaRegistry>>` so `:remove()`
//! can mutably borrow and find the entry by ID.

use std::sync::atomic::{AtomicU64, Ordering};

use mlua::UserData;

use crate::lua::registrar::LuaRegistryHandle;

/// Process-global counter for registrar handle IDs. Bumped once per
/// `register_palette_command` or `register_keybind` call. Unique
/// per-process so the registry's id-based lookup never aliases
/// across plugins or restarts within the same run.
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(0);

/// Allocate the next registrar handle ID. Used by the
/// `register_palette_command` and `register_keybind` capturers in
/// [`crate::lua::api::register`].
pub fn allocate_handle_id() -> u64 {
    NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Lua-facing handle returned by `ttymap.register_palette_command(...)`.
///
/// `:remove()` calls
/// [`LuaRegistry::remove_palette_entry`](crate::lua::registrar::LuaRegistry::remove_palette_entry)
/// — the entry is gone from the next palette open. If the palette
/// is already on screen with the entry visible, selecting it after
/// removal silently no-ops (the registry lookup at execute time
/// returns `None`).
pub struct PaletteCommandHandle {
    id: u64,
    registry: LuaRegistryHandle,
}

impl PaletteCommandHandle {
    pub fn new(id: u64, registry: LuaRegistryHandle) -> Self {
        Self { id, registry }
    }
}

impl UserData for PaletteCommandHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            this.registry.borrow_mut().remove_palette_entry(this.id);
            Ok(())
        });
    }
}

/// Lua-facing handle returned by `ttymap.register_keybind(...)`.
///
/// `:remove()` calls
/// [`LuaRegistry::remove_activation`](crate::lua::registrar::LuaRegistry::remove_activation)
/// — the next keypress for that key falls through to the keymap as
/// if the plugin had never bound it.
pub struct KeybindHandle {
    id: u64,
    registry: LuaRegistryHandle,
}

impl KeybindHandle {
    pub fn new(id: u64, registry: LuaRegistryHandle) -> Self {
        Self { id, registry }
    }
}

impl UserData for KeybindHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("remove", |_, this, _: ()| {
            this.registry.borrow_mut().remove_activation(this.id);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::{Activation, Component, PaletteEntry};
    use crate::lua::registrar::new_lua_registry;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn fake_palette_entry() -> PaletteEntry {
        PaletteEntry {
            label: "Test".to_string(),
            hint: String::new(),
            spawn: std::rc::Rc::new(|_| -> Option<Box<dyn Component>> { None }),
        }
    }

    fn fake_activation() -> Activation {
        Activation {
            code: KeyCode::Char('z'),
            modifiers: KeyModifiers::NONE,
            spawn: std::rc::Rc::new(|_| -> Option<Box<dyn Component>> { None }),
        }
    }

    #[test]
    fn palette_command_handle_remove_drops_entry_from_registry() {
        let registry = new_lua_registry();
        let id = 42;
        registry
            .borrow_mut()
            .add_palette_entry(id, fake_palette_entry());
        assert_eq!(registry.borrow().palette_entry_count(), 1);

        let lua = mlua::Lua::new();
        let ud = lua
            .create_userdata(PaletteCommandHandle::new(id, registry.clone()))
            .unwrap();
        // Two calls in a row must not error (idempotent).
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .expect("remove");
        assert_eq!(
            registry.borrow().palette_entry_count(),
            0,
            "handle:remove() must drop the entry from the registry"
        );
    }

    #[test]
    fn keybind_handle_remove_drops_activation_from_registry() {
        let registry = new_lua_registry();
        let id = 7;
        registry.borrow_mut().add_activation(id, fake_activation());
        assert_eq!(registry.borrow().activation_count(), 1);

        let lua = mlua::Lua::new();
        let ud = lua
            .create_userdata(KeybindHandle::new(id, registry.clone()))
            .unwrap();
        lua.load("local h = ...; h:remove(); h:remove()")
            .call::<()>(ud)
            .expect("remove");
        assert_eq!(registry.borrow().activation_count(), 0);
    }
}
