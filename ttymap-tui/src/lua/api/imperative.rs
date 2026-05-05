//! `ttymap.api` — the (nvim-style) imperative primitives a plugin
//! calls from inside its callbacks (palette `invoke`, `register_keybind`
//! callback, `on_tick`).
//!
//! Sub-tables installed here:
//!
//! - `ttymap.api.card.open(spec) -> CardHandle` — push a focused
//!   [`LuaCardComponent`](crate::lua::bridge::card_component::LuaCardComponent)
//!   onto the compositor stack.
//! - `ttymap.api.palette.open(spec) -> PaletteHandle` — push a
//!   palette provider (a `PaletteComponent` wrapping a
//!   [`LuaPaletteProvider`](crate::lua::bridge::palette_provider::LuaPaletteProvider))
//!   onto the stack. Returning `{ switch = sub_spec }` from the
//!   provider's `execute` swaps the provider in place (sub-mode
//!   transition, no stacking).
//! - `ttymap.api.frame.to_ansi() -> string|nil` — return the latest
//!   `MapFrame` rendered as an ANSI string. Producers (today
//!   `export.lua`) decide where / how to persist it.
//! - `ttymap.api.frame.on_tick(fn)` — subscribe a per-frame callback
//!   (sugar for `ttymap.on_event("tick", fn)`).
//!
//! The activation surfaces (`register_palette_command` /
//! `register_keybind` / `on_event`) live in [`super::register`] —
//! they sit at `ttymap.X`, not under `ttymap.api`, because they're
//! *script-load-time* declarations, not runtime imperative calls.

use mlua::{Lua, Table};

use std::sync::Arc;

use crate::compositor::op::{Op, OpsBuffer};
use crate::lua::capture::{CaptureSlot, EventSubscription};
use crate::lua::host::LuaHostShared;

/// Build the `ttymap.api` sub-table and attach it. Called from
/// [`super::install`] after activation surfaces are registered.
pub(super) fn install(
    lua: &Lua,
    ttymap: &Table,
    tag: &'static str,
    slot: CaptureSlot,
    ops: OpsBuffer,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<()> {
    let api = lua.create_table()?;

    api.set("card", build_card_table(lua, tag, ops.clone())?)?;
    api.set("palette", build_palette_table(lua, tag, ops.clone())?)?;
    api.set("frame", build_frame_table(lua, slot, ops, shared)?)?;

    ttymap.set("api", api)?;
    Ok(())
}

fn build_card_table(lua: &Lua, tag: &'static str, ops: OpsBuffer) -> mlua::Result<Table> {
    let card_api = lua.create_table()?;
    card_api.set(
        "open",
        lua.create_function(
            move |lua, spec: Table| -> mlua::Result<crate::lua::bridge::card_handle::CardHandle> {
                use crate::compositor::CardId;
                use crate::lua::bridge::card_component::LuaCardComponent;
                use crate::lua::bridge::card_handle::CardHandle;
                // Reserve the [`CardId`] at the call site so the
                // handle returned to Lua can target this exact
                // component for close, even though the actual push
                // applies when the App drains the `OpsBuffer` next
                // iteration.
                let id = CardId::next();
                // Build the component on the **same** Lua VM that ran
                // `card.open` — i.e. the setup state. The spec's
                // callbacks (`render`, `handle_key`, …) capture
                // upvalues in this state, so the per-window Lua handle
                // must be a clone of it (cheap Arc bump, no copy of the
                // VM). When `LuaCardComponent` later calls into those
                // callbacks, the same upvalue scope is in scope.
                let component = LuaCardComponent::from_spec(lua.clone(), spec, tag)?;
                ops.borrow_mut().push(Op::Push {
                    id,
                    component: Box::new(component) as Box<dyn crate::compositor::Component>,
                });
                Ok(CardHandle::new(id, ops.clone()))
            },
        )?,
    )?;
    Ok(card_api)
}

fn build_palette_table(lua: &Lua, tag: &'static str, ops: OpsBuffer) -> mlua::Result<Table> {
    let palette_api = lua.create_table()?;
    palette_api.set(
        "open",
        lua.create_function(
            move |lua,
                  spec: Table|
                  -> mlua::Result<crate::lua::bridge::palette_handle::PaletteHandle> {
                use crate::compositor::CardId;
                use crate::lua::bridge::palette_handle::PaletteHandle;
                use crate::lua::bridge::palette_provider::LuaPaletteProvider;
                // Reserve the id up-front so the returned [`PaletteHandle`]
                // can target this exact PaletteComponent for close.
                let id = CardId::next();
                // Build the provider on the **same** Lua VM that ran
                // `palette.open` — the setup state. The spec's
                // callbacks (`filter`, `items`, `execute`, …) capture
                // upvalues there, so the per-provider Lua handle must
                // be a clone of it (cheap Arc bump).
                let provider = LuaPaletteProvider::from_spec(lua.clone(), spec, tag)?;
                let palette = crate::palette::PaletteComponent::with_provider(Box::new(provider));
                ops.borrow_mut().push(Op::Push {
                    id,
                    component: Box::new(palette) as Box<dyn crate::compositor::Component>,
                });
                Ok(PaletteHandle::new(id, ops.clone()))
            },
        )?,
    )?;
    Ok(palette_api)
}

fn build_frame_table(
    lua: &Lua,
    slot: CaptureSlot,
    _ops: OpsBuffer,
    shared: Arc<LuaHostShared>,
) -> mlua::Result<Table> {
    let frame_api = lua.create_table()?;

    // `to_ansi()` returns the latest [`MapFrame`] rendered as an
    // ANSI string, or `nil` if no frame has arrived yet. Callers
    // (today: bundled `export.lua`) decide where + how to persist
    // it — the host hands over the bytes and gets out of the way.
    let shared_for_ansi = shared;
    frame_api.set(
        "to_ansi",
        lua.create_function(move |_, _: ()| -> mlua::Result<Option<String>> {
            let Ok(slot) = shared_for_ansi.current_frame.lock() else {
                return Ok(None);
            };
            Ok(slot.as_ref().map(|f| f.to_ansi()))
        })?,
    )?;

    // `on_tick` is a thin sugar for `on_event("tick", fn)` — kept
    // because the existing plugin set + docs use it everywhere and
    // it reads more naturally for the per-frame use case. New
    // event surfaces should use `ttymap.on_event` directly.
    frame_api.set(
        "on_tick",
        lua.create_function(move |lua, callback: mlua::Function| -> mlua::Result<()> {
            let key = lua.create_registry_value(callback)?;
            slot.borrow_mut()
                .event_subscriptions
                .push(EventSubscription {
                    event_name: "tick",
                    callback: key,
                });
            Ok(())
        })?,
    )?;
    Ok(frame_api)
}
