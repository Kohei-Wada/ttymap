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
//! [`LuaBridgeHandle::try_call`] for the dispatch shape. [`load_chunk`]
//! is the per-plugin loader the registrar walks call.
//!
//! [`LuaCardComponent`]: super::card_component::LuaCardComponent
//! [`LuaPaletteProvider`]: super::palette_provider::LuaPaletteProvider

use mlua::{Lua, RegistryKey, Table};

use crate::lua::capture::{CaptureSlot, CapturedRegistration};

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

/// Run `source` in the shared `lua` VM and return the registrations
/// the script made via `ttymap.register_*` / `ttymap.on_event` /
/// `ttymap.api.frame.on_tick`.
///
/// nvim-style: the script's existence in `<runtime>/plugin/` is the
/// registration. Identity = file stem (passed as `chunk_name`). The
/// script participates in the host loop by calling some combination of:
///
/// - `ttymap.api.frame.on_tick(fn)` — per-frame work (paint markers,
///   drain async fetches, etc.)
/// - `ttymap.register_palette_command({label, invoke})` — palette row
/// - `ttymap.register_keybind(key, callback)` — top-level keybind
/// - `ttymap.on_event(name, fn)` — generic event subscription
///
/// At least one of those calls is required; a script that subscribes
/// to nothing surfaces as an `mlua::Error` here.
///
/// `slot` is the per-VM capture buffer the `ttymap.register_*`
/// closures write into. Caller is responsible for draining it before
/// each call (the function clears stale captures defensively).
///
/// `chunk_name` is reported in Lua error messages — pass the plugin's
/// file stem so a stack trace pinpoints the script. The same value
/// is set as `slot.current_plugin` for the duration of `exec` so
/// `on_tick` / `on_event` capturers can attribute their bus
/// subscriptions to this plugin.
pub fn load_chunk(
    lua: &Lua,
    source: &str,
    chunk_name: &'static str,
    slot: &CaptureSlot,
) -> mlua::Result<CapturedRegistration> {
    // Stack-style save: a plugin may recursively `require` another
    // plugin, in which case we re-enter `load_chunk` while an outer
    // load is mid-execution. Take the outer state aside, run the
    // inner load with a fresh slot, then restore the outer state so
    // the outer load's `register_*` capture continues correctly.
    // Production today never recurses (plugins only require libs),
    // but the cost of the extra `mem::take` pair is negligible and
    // the safety it buys is worth it.
    let outer = std::mem::take(&mut *slot.borrow_mut());
    slot.borrow_mut().current_plugin = Some(chunk_name);

    let exec_result = lua.load(source).set_name(chunk_name).exec();

    // Drain whatever the inner load captured, regardless of success
    // — we still want to restore `outer` even if exec errored, so
    // an outer plugin's pcall can recover.
    let inner = std::mem::take(&mut *slot.borrow_mut());
    *slot.borrow_mut() = outer;

    exec_result?;

    let has_surface = !inner.palette_commands.is_empty()
        || !inner.keybinds.is_empty()
        || inner.events_registered > 0;
    if !has_surface {
        return Err(mlua::Error::external(
            "script did not call any ttymap registration API \
             (ttymap.on_event, ttymap.api.frame.on_tick, \
             ttymap.register_palette_command, or ttymap.register_keybind)",
        ));
    }
    Ok(inner)
}
