//! Shared Lua-bridge plumbing for adapter types
//! ([`LuaCardComponent`], [`LuaPaletteProvider`], future ones).
//!
//! Every adapter does the same two things:
//! 1. **Construction** — clone the shared `Lua` VM (cheap Arc bump),
//!    stash the spec table in the Lua registry so dispatch hooks
//!    can re-fetch it cheaply.
//! 2. **Dispatch** — for each trait method, look up `module[name]`,
//!    call it if present, log + recover if absent or erroring.
//!
//! [`LuaBridgeHandle`] owns the registry handle + log tag and provides
//! [`LuaBridgeHandle::try_call`] for the dispatch shape.
//!
//! [`LuaCardComponent`]: super::card_component::LuaCardComponent
//! [`LuaPaletteProvider`]: super::palette_provider::LuaPaletteProvider

use mlua::{Lua, RegistryKey, Table};

/// Per-adapter Lua state + the registry handle for the dispatch
/// table. Both adapters (Component, PaletteProvider) compose this
/// instead of carrying `lua` and `module` as separate fields.
pub struct LuaBridgeHandle {
    /// Identifier used in log warnings (`lua[wiki]: poll() failed:
    /// …`). For bundled plugins this is the file stem; for user
    /// plugins it's the leaked stem from the registration walker.
    log_tag: &'static str,
    lua: Lua,
    module: RegistryKey,
}

impl LuaBridgeHandle {
    /// Build a handle around `table` evaluated inside `lua`. The
    /// caller has already inspected the table — read metadata, or
    /// drilled into a sub-table — and is handing it over for
    /// long-lived dispatch.
    pub fn new(lua: Lua, table: Table, log_tag: &'static str) -> mlua::Result<Self> {
        let module = lua.create_registry_value(table)?;
        Ok(Self {
            lua,
            module,
            log_tag,
        })
    }

    /// Re-fetch the registered table. Cheap (registry lookup).
    pub fn module(&self) -> mlua::Result<Table> {
        self.lua.registry_value(&self.module)
    }

    /// Direct `Lua` access for callers that need `Lua::scope` or
    /// must build per-call helper tables — the per-frame plugin
    /// `loop` callback's scoped `MapApi` userdata is one consumer.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn log_tag(&self) -> &'static str {
        self.log_tag
    }

    /// Try to call `module[method](args)` and return its result.
    /// See [`CallOutcome`] for the three possible states. The
    /// `Errored` arm has already been logged.
    ///
    /// The missing-vs-errored split is intentional: an adapter may
    /// want different recovery for "plugin opted out of this hook"
    /// vs "plugin tried but threw" — e.g.
    /// [`LuaCardComponent`](super::card_component::LuaCardComponent)'s
    /// `handle_key` maps the former to `KeyAction::Ignore`
    /// (forward to base) and the latter to `KeyAction::Consume`
    /// (don't leak buggy keys).
    pub fn try_call<A, R>(&self, method: &str, args: A) -> CallOutcome<R>
    where
        A: mlua::IntoLuaMulti,
        R: mlua::FromLuaMulti,
    {
        let result: mlua::Result<Option<R>> = (|| {
            let module = self.module()?;
            let f: Option<mlua::Function> = module.get(method).ok();
            match f {
                Some(f) => Ok(Some(f.call(args)?)),
                None => Ok(None),
            }
        })();
        match result {
            Ok(Some(r)) => CallOutcome::Ok(r),
            Ok(None) => CallOutcome::Missing,
            Err(e) => {
                log::warn!("lua[{}]: {}() failed: {}", self.log_tag, method, e);
                CallOutcome::Errored
            }
        }
    }
}

/// Outcome of [`LuaBridgeHandle::try_call`].
pub enum CallOutcome<R> {
    /// The method exists and returned a value.
    Ok(R),
    /// `module[method]` is absent or not a function.
    Missing,
    /// The call errored; a `log::warn!` has been emitted with the
    /// handle's log tag and method name as context.
    Errored,
}
