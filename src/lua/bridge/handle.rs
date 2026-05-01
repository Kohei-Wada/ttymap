//! Shared Lua-bridge plumbing for adapter types
//! ([`LuaWindowComponent`], [`LuaPaletteProvider`], future ones).
//!
//! Every adapter does the same two things:
//! 1. **Construction** — spin up a fresh `Lua` VM, install the
//!    `ttymap.*` host services, evaluate the source, and stash the
//!    resulting module table in the Lua registry so dispatch hooks
//!    can re-fetch it cheaply.
//! 2. **Dispatch** — for each trait method, look up `module[name]`,
//!    call it if present, log + recover if absent or erroring.
//!
//! [`LuaHandle`] owns the registry handle + log tag and provides
//! [`LuaHandle::try_call`] for the dispatch shape. [`fresh_load`] is
//! the one-shot construction helper.
//!
//! [`LuaWindowComponent`]: super::window_component::LuaWindowComponent
//! [`LuaPaletteProvider`]: super::palette_provider::LuaPaletteProvider

use std::sync::Arc;

use mlua::{Lua, RegistryKey, Table};

use crate::lua::new_lua;
use crate::lua::ttymap as host;

/// Per-adapter Lua state + the registry handle for the dispatch
/// table. Both adapters (Component, PaletteProvider) compose this
/// instead of carrying `lua` and `module` as separate fields.
pub struct LuaHandle {
    /// Identifier used in log warnings (`lua[wiki]: poll() failed:
    /// …`). For bundled plugins this is the file stem; for user
    /// plugins it's the leaked stem from the registration walker.
    /// Same value the component returns from
    /// [`crate::compositor::Component::dedup_tag`], so logs and
    /// stack identity agree.
    log_tag: &'static str,
    lua: Lua,
    module: RegistryKey,
}

impl LuaHandle {
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
    /// [`LuaWindowComponent`](super::window_component::LuaWindowComponent)'s
    /// `handle_event` maps the former to `KeyAction::Ignore`
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

/// Outcome of [`LuaHandle::try_call`].
pub enum CallOutcome<R> {
    /// The method exists and returned a value.
    Ok(R),
    /// `module[method]` is absent or not a function.
    Missing,
    /// The call errored; a `log::warn!` has been emitted with the
    /// handle's log tag and method name as context.
    Errored,
}

/// Build a fresh Lua state, install host services, run `source`, and
/// hand back the captured registration spec for the caller to inspect
/// before constructing a [`LuaHandle`].
///
/// The script self-registers by calling at least one
/// `ttymap.register_*` API. Two valid shapes:
///
/// 1. **Componented plugin**: calls `register_plugin` (which captures
///    the spec table), optionally with activation surfaces
///    (`register_palette_command` / `register_keybind` /
///    `register_footer_hint`). Palette providers are *not* declared
///    at top level anymore; they're pushed dynamically via
///    `ttymap.api.palette.open(spec)` from inside an activation
///    callback.
/// 2. **Pure-action plugin**: no spec, but at least one activation
///    surface — typically a `register_palette_command` whose
///    `invoke` calls a fire-and-forget host API like
///    `ttymap.api.frame.export()` or `ttymap.map:jump(...)`.
///    `export.lua` is the canonical example.
///
/// A script that calls *none* of the registration APIs surfaces as
/// an `mlua::Error` here.
///
/// - `chunk_name` is reported in Lua error messages; pass the file
///   stem so a stack trace pinpoints the script.
/// - `host_tag` is the HTTP User-Agent suffix for `ttymap.http`.
pub fn fresh_load(
    source: &str,
    chunk_name: &str,
    host_tag: &'static str,
    shared: Arc<host::LuaHostShared>,
) -> mlua::Result<(Lua, host::CapturedRegistration, host::LuaHostHandles)> {
    let lua = new_lua();
    let slot = host::new_capture_slot();
    let handles = host::install(&lua, host_tag, shared, slot.clone())?;
    lua.load(source).set_name(chunk_name).exec()?;
    let captured = std::mem::take(&mut *slot.borrow_mut());
    let has_surface = !captured.palette_commands.is_empty()
        || !captured.keybinds.is_empty()
        || !captured.footer_hints.is_empty();
    if captured.spec.is_none() && !has_surface {
        return Err(mlua::Error::external(
            "script did not call any ttymap.register_* API \
             (register_plugin, register_palette_command, \
             register_keybind, or register_footer_hint)",
        ));
    }
    Ok((lua, captured, handles))
}
