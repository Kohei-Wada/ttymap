//! Top-level `ttymap.X` registration functions a script calls during
//! its setup body to declare keybinds, palette rows, and event
//! subscriptions.
//!
//! `register_palette_command` / `register_keybind` push entries
//! **directly** into the [`LuaRegistry`] (no deferred capture, no
//! per-script slot). The host doesn't track which script registered
//! what; "plugin" is purely a Lua-side organisational unit (one .lua
//! file's worth of `register_*` calls). Each call returns a handle
//! whose `:remove()` drops the registration.
//!
//! `on_event` likewise subscribes directly against the
//! [`EventBus`](ttymap_core::event::EventBus) and returns an
//! [`EventHandle`].

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};

use crossterm::event::{KeyCode, KeyModifiers};
use mlua::{Lua, Table};

use crate::bridge::event_handle::EventHandle;
use crate::bridge::registrar_handle::{KeybindHandle, PaletteCommandHandle, allocate_handle_id};
use crate::host::{HelpEntry, LuaHostShared};
use crate::registrar::LuaRegistryHandle;
use crate::tick::TickRegistry;
use ttymap_core::event::{Event, EventBus, Level};
use ttymap_tui::compositor::{Activation, PaletteEntry, SpawnComponent};

pub(super) fn install(
    lua: &Lua,
    ttymap: &Table,
    bus: Rc<EventBus>,
    ticks: Rc<TickRegistry>,
    registry: LuaRegistryHandle,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<()> {
    install_register_palette_command(lua, ttymap, registry.clone(), shared)?;
    install_register_keybind(lua, ttymap, registry)?;
    install_on_event(lua, ttymap, bus, ticks)?;
    Ok(())
}

fn install_register_palette_command(
    lua: &Lua,
    ttymap: &Table,
    registry: LuaRegistryHandle,
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
    registry: LuaRegistryHandle,
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

/// Intern an event name to a `&'static str`. The bus's bucket key
/// is `&'static str` (cheap copy + comparison), but Lua hands us a
/// `String` per call. Naively `Box::leak`ing on every call would
/// leak unbounded memory if the same name is registered many times
/// (a plugin churning `on_event(name, fn)` / `:remove()` cycles).
/// The interner ensures **one leak per distinct name** for the
/// program lifetime; repeat calls reuse the existing static.
///
/// Single-threaded in practice (Lua main thread only) but the
/// `Mutex` keeps it `Sync` for free.
fn intern_event_name(name: &str) -> &'static str {
    static INTERNED: LazyLock<Mutex<HashSet<&'static str>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));
    let mut set = INTERNED.lock().expect("event-name interner poisoned");
    if let Some(&existing) = set.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// `ttymap.on_event(name, fn) -> EventHandle` — generic pub/sub
/// subscription. Routes by name:
///
/// - `"tick"` goes to the [`TickRegistry`] (per-frame callback with
///   a `map` arg) — same path as `ttymap.api.frame.on_tick(fn)`.
/// - everything else subscribes against the [`EventBus`], with the
///   Lua callback wrapped in a Rust closure that captures the
///   `mlua::Lua` + `RegistryKey` and converts the [`Event`] payload
///   to the typed Lua arg the plugin expects (today: `notify` →
///   `{ message, level }`).
///
/// Returns a handle whose `:remove()` drops the exact subscriber
/// from the registry it landed in.
fn install_on_event(
    lua: &Lua,
    ttymap: &Table,
    bus: Rc<EventBus>,
    ticks: Rc<TickRegistry>,
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

                if event_name == "tick" {
                    // `tick` payload is a per-frame `map` table built
                    // in `TickRegistry::dispatch` — can't ride the
                    // typed-Event bus.
                    let key = lua.create_registry_value(callback)?;
                    let id = ticks.subscribe(lua.clone(), key);
                    let ticks_for_remove = Rc::clone(&ticks);
                    return Ok(EventHandle::new(Rc::new(move || {
                        ticks_for_remove.remove(id);
                    })));
                }

                let interned = intern_event_name(&event_name);
                let key = Rc::new(lua.create_registry_value(callback)?);
                let lua_for_closure = lua.clone();
                let key_for_closure = Rc::clone(&key);
                let id = bus.subscribe(interned, move |event| {
                    let f: mlua::Function =
                        match lua_for_closure.registry_value::<mlua::Function>(&key_for_closure) {
                            Ok(f) => f,
                            Err(e) => {
                                log::warn!(
                                    "lua: {} subscriber registry lookup failed: {}",
                                    interned,
                                    e,
                                );
                                return;
                            }
                        };
                    if let Err(e) = call_lua_with_event(&f, &lua_for_closure, event) {
                        log::warn!("lua: {} subscriber failed: {}", interned, e);
                    }
                });

                let bus_for_remove = Rc::clone(&bus);
                Ok(EventHandle::new(Rc::new(move || {
                    bus_for_remove.remove(interned, id);
                })))
            },
        )?,
    )
}

/// Build the Lua arg tuple for `event` and call `f`. The bus
/// currently carries one variant (`Notify`); future structured
/// events get their own arm here when a subscriber needs them.
fn call_lua_with_event(f: &mlua::Function, lua: &Lua, event: &Event) -> mlua::Result<()> {
    match event {
        Event::Notify { message, level } => {
            let t = lua.create_table()?;
            t.set("message", message.as_str())?;
            t.set(
                "level",
                match level {
                    Level::Info => "info",
                    Level::Warn => "warn",
                    Level::Error => "error",
                },
            )?;
            f.call::<()>(t)
        }
    }
}
