//! Activation-surface registration — top-level `ttymap.X` functions a
//! plugin's setup body calls to declare keybinds, palette rows, and
//! event subscriptions.
//!
//! All three are *capturers*: they don't run callbacks immediately,
//! they push a [`PaletteCommandSpec`] / [`KeybindSpec`] /
//! [`EventSubscription`] into the shared [`CaptureSlot`] which the
//! host inspects after the script finishes loading.

use mlua::{Lua, Table};

use super::{CaptureSlot, EventSubscription, KeybindSpec, PaletteCommandSpec};

/// Install the activation-surface entries onto an existing `ttymap`
/// table. Called from [`super::install`] before the imperative
/// primitives go on.
pub(super) fn install(lua: &Lua, ttymap: &Table, slot: CaptureSlot) -> mlua::Result<()> {
    install_register_palette_command(lua, ttymap, slot.clone())?;
    install_register_keybind(lua, ttymap, slot.clone())?;
    install_on_event(lua, ttymap, slot)?;
    Ok(())
}

fn install_register_palette_command(
    lua: &Lua,
    ttymap: &Table,
    slot: CaptureSlot,
) -> mlua::Result<()> {
    ttymap.set(
        "register_palette_command",
        lua.create_function(move |lua, spec: Table| -> mlua::Result<()> {
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
            slot.borrow_mut().palette_commands.push(PaletteCommandSpec {
                label,
                hint,
                invoke: invoke_key,
            });
            Ok(())
        })?,
    )
}

fn install_register_keybind(lua: &Lua, ttymap: &Table, slot: CaptureSlot) -> mlua::Result<()> {
    ttymap.set(
        "register_keybind",
        lua.create_function(
            move |lua, (key, callback): (String, mlua::Function)| -> mlua::Result<()> {
                let Some(c) = key.chars().next() else {
                    return Err(mlua::Error::external(
                        "ttymap.register_keybind: key must be a non-empty string",
                    ));
                };
                let callback_key = lua.create_registry_value(callback)?;
                slot.borrow_mut().keybinds.push(KeybindSpec {
                    key: c,
                    callback: callback_key,
                });
                Ok(())
            },
        )?,
    )
}

/// `ttymap.on_event(name, fn)` — generic pub/sub subscription.
/// Lower into a [`EventSubscription`] keyed by the leaked event
/// name; the host walks them at register time and pushes one
/// [`Subscriber`](crate::lua::registry::Subscriber) into the
/// matching [`LuaEventBus`](crate::lua::LuaEventBus) bucket.
///
/// The leak is bounded by `(unique event names) × plugins`,
/// happens at register time only, and produces `&'static str`
/// (which the bus needs as a HashMap key matching plugin-name
/// and source-text leaks done elsewhere in `register_plugins_in`).
///
/// `ttymap.api.frame.on_tick(fn)` (in [`super::imperative`]) is
/// sugar for `ttymap.on_event("tick", fn)` — same Subscriber shape,
/// same dispatch path, just a different surface for the common case.
fn install_on_event(lua: &Lua, ttymap: &Table, slot: CaptureSlot) -> mlua::Result<()> {
    ttymap.set(
        "on_event",
        lua.create_function(
            move |lua, (event_name, callback): (String, mlua::Function)| -> mlua::Result<()> {
                if event_name.is_empty() {
                    return Err(mlua::Error::external(
                        "ttymap.on_event: event name must be a non-empty string",
                    ));
                }
                let leaked: &'static str = Box::leak(event_name.into_boxed_str());
                let key = lua.create_registry_value(callback)?;
                slot.borrow_mut()
                    .event_subscriptions
                    .push(EventSubscription {
                        event_name: leaked,
                        callback: key,
                    });
                Ok(())
            },
        )?,
    )
}
