//! Activation-surface registration — top-level `ttymap.X` functions a
//! plugin's setup body calls to declare keybinds, palette rows, and
//! event subscriptions.
//!
//! `register_palette_command` / `register_keybind` are *capturers*:
//! they push a [`PaletteCommandSpec`] / [`KeybindSpec`] into the
//! shared [`CaptureSlot`] which the host inspects after the script
//! finishes loading.
//!
//! `on_event` is **not** a capturer — it subscribes directly against
//! the [`EventBus`] at call time and returns an [`EventHandle`] so
//! plugins can `:remove()` later. The slot's `events_registered`
//! counter is bumped so the loader's "must subscribe to something"
//! gate still sees event-only plugins.

use std::rc::Rc;

use mlua::{Lua, Table};

use crate::event::EventBus;
use crate::lua::bridge::event_handle::EventHandle;
use crate::lua::bridge::registrar_handle::{
    KeybindHandle, PaletteCommandHandle, allocate_handle_id,
};
use crate::lua::capture::{CaptureSlot, KeybindSpec, PaletteCommandSpec};
use crate::lua::registrar::PluginRegistryHandle;

/// Install the activation-surface entries onto an existing `ttymap`
/// table. Called from [`super::install`] before the imperative
/// primitives go on.
pub(super) fn install(
    lua: &Lua,
    ttymap: &Table,
    slot: CaptureSlot,
    bus: Rc<EventBus>,
    registry: PluginRegistryHandle,
) -> mlua::Result<()> {
    install_register_palette_command(lua, ttymap, slot.clone(), registry.clone())?;
    install_register_keybind(lua, ttymap, slot.clone(), registry)?;
    install_on_event(lua, ttymap, slot, bus)?;
    Ok(())
}

fn install_register_palette_command(
    lua: &Lua,
    ttymap: &Table,
    slot: CaptureSlot,
    registry: PluginRegistryHandle,
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
                let invoke_key = lua.create_registry_value(invoke)?;
                let id = allocate_handle_id();
                slot.borrow_mut().palette_commands.push(PaletteCommandSpec {
                    id,
                    label,
                    hint,
                    invoke: invoke_key,
                });
                Ok(PaletteCommandHandle::new(id, registry.clone()))
            },
        )?,
    )
}

fn install_register_keybind(
    lua: &Lua,
    ttymap: &Table,
    slot: CaptureSlot,
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
                let callback_key = lua.create_registry_value(callback)?;
                let id = allocate_handle_id();
                slot.borrow_mut().keybinds.push(KeybindSpec {
                    id,
                    key: c,
                    callback: callback_key,
                });
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
/// str` key requirement is met. Bounded by `(unique event names) ×
/// plugins`; happens at register time only, matching plugin-name
/// and source-text leaks elsewhere in `register_plugins_in`.
///
/// `ttymap.api.frame.on_tick(fn)` (in [`super::imperative`]) is
/// sugar for `ttymap.on_event("tick", fn)` — same Subscriber shape,
/// same dispatch path, just a different surface for the common case.
fn install_on_event(
    lua: &Lua,
    ttymap: &Table,
    slot: CaptureSlot,
    bus: Rc<EventBus>,
) -> mlua::Result<()> {
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
                let plugin = slot.borrow().current_plugin.unwrap_or("(unknown)");
                let key = lua.create_registry_value(callback)?;
                let id = bus.subscribe_lua(leaked, plugin, lua.clone(), key);
                slot.borrow_mut().events_registered += 1;
                Ok(EventHandle::new(bus.clone(), leaked, id))
            },
        )?,
    )
}
