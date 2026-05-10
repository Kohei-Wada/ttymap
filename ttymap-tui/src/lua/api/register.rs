//! Top-level `ttymap.X` registration functions a script calls during
//! its setup body to declare keybinds, palette rows, and event
//! subscriptions.
//!
//! `register_palette_command` / `register_keybind` push entries
//! **directly** into the [`PluginRegistry`] (no deferred capture, no
//! per-script slot). The host doesn't track which script registered
//! what; "plugin" is purely a Lua-side organisational unit (one .lua
//! file's worth of `register_*` calls). Each call returns a handle
//! whose `:remove()` drops the registration.
//!
//! `on_event` likewise subscribes directly against the
//! [`EventBus`](crate::event::EventBus) and returns an
//! [`EventHandle`].

use std::rc::Rc;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use mlua::{Lua, Table};

use crate::compositor::{Activation, PaletteEntry, SpawnComponent};
use crate::event::EventBus;
use crate::lua::bridge::event_handle::EventHandle;
use crate::lua::bridge::registrar_handle::{
    KeybindHandle, PaletteCommandHandle, allocate_handle_id,
};
use crate::lua::host::{HelpEntry, LuaHostShared};
use crate::lua::registrar::PluginRegistryHandle;

pub(super) fn install(
    lua: &Lua,
    ttymap: &Table,
    bus: Rc<EventBus>,
    registry: PluginRegistryHandle,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<()> {
    install_register_palette_command(lua, ttymap, registry.clone(), shared)?;
    install_register_keybind(lua, ttymap, registry)?;
    install_on_event(lua, ttymap, bus)?;
    Ok(())
}

fn install_register_palette_command(
    lua: &Lua,
    ttymap: &Table,
    registry: PluginRegistryHandle,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<()> {
    ttymap.set(
        "register_palette_command",
        lua.create_function(
            move |lua, spec: Table| -> mlua::Result<PaletteCommandHandle> {
                let label: String = spec.get("label").map_err(|_| {
                    mlua::Error::external("ttymap.register_palette_command: spec.label is required")
                })?;
                let hint: String = spec.get("hint").unwrap_or_default();
                let invoke: mlua::Function = spec.get("invoke").map_err(|_| {
                    mlua::Error::external(
                        "ttymap.register_palette_command: spec.invoke (a function) is required",
                    )
                })?;
                let invoke_key = Rc::new(lua.create_registry_value(invoke)?);
                let id = allocate_handle_id();

                // Build the activation factory inline. On invoke,
                // re-fetch the callback from the Lua registry and
                // call it; errors are logged + swallowed so a buggy
                // callback doesn't take the host down.
                let lua_for_factory = lua.clone();
                let invoke_for_factory = invoke_key.clone();
                let factory: SpawnComponent = Rc::new(move |_ctx| {
                    let f: mlua::Function =
                        match lua_for_factory.registry_value(&invoke_for_factory) {
                            Ok(f) => f,
                            Err(e) => {
                                log::warn!("lua: palette callback registry lookup failed: {}", e);
                                return None;
                            }
                        };
                    if let Err(e) = f.call::<mlua::Value>(()) {
                        log::warn!("lua: palette callback failed: {}", e);
                    }
                    None
                });

                // Surface to help cheatsheet IFF the entry has a
                // hint (the keybind string). Palette-only entries
                // don't go on the cheatsheet.
                if !hint.is_empty() {
                    shared.push_help_entry(HelpEntry {
                        key: hint.clone(),
                        label: label.clone(),
                    });
                }

                registry.borrow_mut().add_palette_entry(
                    id,
                    PaletteEntry {
                        label,
                        hint,
                        spawn: factory,
                    },
                );
                Ok(PaletteCommandHandle::new(id, registry.clone()))
            },
        )?,
    )
}

fn install_register_keybind(
    lua: &Lua,
    ttymap: &Table,
    registry: PluginRegistryHandle,
) -> mlua::Result<()> {
    ttymap.set(
        "register_keybind",
        lua.create_function(
            move |lua, (key, callback): (String, mlua::Function)| -> mlua::Result<KeybindHandle> {
                let Some(c) = key.chars().next() else {
                    return Err(mlua::Error::external(
                        "ttymap.register_keybind: key must be a non-empty string",
                    ));
                };
                let callback_key = Rc::new(lua.create_registry_value(callback)?);
                let id = allocate_handle_id();

                // Build the activation factory inline (same shape as
                // palette command's).
                let lua_for_factory = lua.clone();
                let cb_for_factory = callback_key.clone();
                let factory: SpawnComponent = Rc::new(move |_ctx| {
                    let f: mlua::Function = match lua_for_factory.registry_value(&cb_for_factory) {
                        Ok(f) => f,
                        Err(e) => {
                            log::warn!("lua: keybind callback registry lookup failed: {}", e);
                            return None;
                        }
                    };
                    if let Err(e) = f.call::<mlua::Value>(()) {
                        log::warn!("lua: keybind callback failed: {}", e);
                    }
                    None
                });

                registry.borrow_mut().add_activation(
                    id,
                    Activation {
                        code: KeyCode::Char(c),
                        modifiers: KeyModifiers::NONE,
                        spawn: factory,
                    },
                );
                Ok(KeybindHandle::new(id, registry.clone()))
            },
        )?,
    )
}

/// `ttymap.on_event(name, fn) -> EventHandle` — generic pub/sub
/// subscription. Subscribes directly against the
/// [`EventBus`](crate::event::EventBus) at call time and returns a
/// handle whose `:remove()` drops the exact subscriber.
///
/// The event name is leaked (`Box::leak`) so the bus's `&'static
/// str` key requirement is met.
fn install_on_event(lua: &Lua, ttymap: &Table, bus: Rc<EventBus>) -> mlua::Result<()> {
    ttymap.set(
        "on_event",
        lua.create_function(
            move |lua,
                  (event_name, callback): (String, mlua::Function)|
                  -> mlua::Result<EventHandle> {
                if event_name.is_empty() {
                    return Err(mlua::Error::external(
                        "ttymap.on_event: event name must be a non-empty string",
                    ));
                }
                let leaked: &'static str = Box::leak(event_name.into_boxed_str());
                let key = lua.create_registry_value(callback)?;
                let id = bus.subscribe_lua(leaked, lua.clone(), key);
                Ok(EventHandle::new(bus.clone(), leaked, id))
            },
        )?,
    )
}
