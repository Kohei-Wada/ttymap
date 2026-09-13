//! Builder for the runtime `ttymap` Lua global — the API surface every
//! plugin script reaches into.
//!
//! `ttymap` is a Lua **table** (not a single userdata) whose fields
//! are domain-namespaced userdatas. Each namespace owns the slice of
//! state its methods need; nothing forces every plugin's call to walk
//! a kitchen-sink struct. Adding a new domain (orbit propagation,
//! logging, scheduling, …) is one new namespace, no churn on existing
//! ones.
//!
//! Submodules: one per Lua namespace (`ttymap.<X>`).
//! - [`http`], [`json`], [`sgp4`] — top-level userdata namespaces
//! - [`map`] — `ttymap.map` userdata (`HostMap`) **and** the per-frame
//!   `map` table handed to `on_tick` callbacks (`make_map_table`,
//!   wrapping the host-side [`crate::MapApi`])
//! - `config`, `help`, `log`, `tile` — host-state namespaces
//! - `imperative` — `ttymap.api.{card,palette,frame}` cluster
//! - `register` — setup-time `ttymap.register_*` / `on_event` capture
//!
//! Surface today:
//!
//! ```text
//! ttymap.http   :fetch(url) -> Job          background HTTP GET (UTF-8 body).
//!                                            Job: :try_take() polls; :cancel()
//!                                            disposes (idempotent — buffered
//!                                            body becomes unreachable from
//!                                            try_take after cancel).
//! ttymap.http   :fetch_cached(url, ttl) -> Job  disk-cached GET; on HTTP
//!                                            error falls back to the
//!                                            stale on-disk copy if any
//! ttymap.http   :url_encode(s) -> string    RFC 3986 query encoding
//! ttymap.map    :jump(lon, lat)             recentre the map (fire-and-forget)
//! ttymap.map    :zoom(level)                set zoom directly (clamped to map's
//!                                            allowed range; fire-and-forget)
//! ttymap.map    :zoom() -> level             current zoom (no-arg getter form),
//!                                            refreshed per dispatch
//! ttymap.map    :fly_to(lon, lat, zoom)     composite recenter + zoom in one
//!                                            dispatch (avoids the intermediate
//!                                            new-centre / old-zoom frame)
//! ttymap.map    :center() -> lon, lat       latest centre, refreshed per dispatch
//! ttymap.json   :parse(s) -> value|nil      JSON → Lua tables (errors → nil)
//! ttymap.json   :stringify(value) -> string Lua → JSON (mixed-key tables /
//!                                            function / userdata → error;
//!                                            NaN / Infinity → null;
//!                                            empty `{}` → empty object)
//! ttymap.sgp4   :parse_tle(text) -> handle  parse a TLE for SGP4 propagation
//! ttymap.sgp4   :parse_tles(text) -> array  parse a multi-TLE block (groups)
//! ttymap.sgp4   :propagate(h[, t]) -> table propagate a handle to unix time t
//! ttymap.sgp4   :propagate_batch(hs[, t])   batch propagate (Starlink-scale)
//! ttymap.tile   :attribution() -> string?   active tile provider's attribution
//! ttymap.config (currently empty — endpoint config lives in plugin
//!                Lua libs at runtime/lua/ttymap/<name>.lua, not here)
//! ttymap.help   :keymap_entries() -> list   built-in keymap rows for help
//! ttymap.help   :palette_entries() -> list  per-plugin metadata for help
//! ttymap.version                            workspace version string (e.g. "0.2.0")
//! ttymap.log    :info(msg) / :warn(msg) / :error(msg)
//!                                            forward to host log at
//!                                            target `lua`
//! ttymap.storage:open(ns) -> Store           per-namespace persistent KV
//!                                            under XDG data dir; values
//!                                            are JSON-encoded via the same
//!                                            `lua_to_json` rules as
//!                                            `ttymap.json:stringify`.
//!                                            Store: :get(k, default) -> v,
//!                                            :set(k, v), :delete(k).
//!                                            Atomic write (temp + rename);
//!                                            corrupt / missing files
//!                                            return `default`. Namespace
//!                                            and key match `[A-Za-z0-9_-]+`.
//! ttymap.api.card.open(spec) -> Handle    push a focused window
//!                                            (LuaCardComponent) onto
//!                                            the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.palette.open(spec) -> Handle   push a palette provider
//!                                            onto the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.frame.to_ansi() -> string?    latest frame as ANSI bytes,
//!                                            or nil if no frame yet (caller
//!                                            decides where to persist)
//! ttymap.api.frame.on_tick(callback)        register a per-frame callback
//!                                            (called with `MapApi`); multiple
//!                                            calls per script are stacked
//! ttymap.notify(msg [, opts])               post a transient status message;
//!                                            opts.level is `info` (default) /
//!                                            `warn` / `error`. Lowers to
//!                                            `Op::Publish(Event::Notify)`;
//!                                            the bundled `notify.lua`
//!                                            subscriber renders recent
//!                                            entries in a corner.
//! ttymap.on_event("notify", fn)              subscribe to bus events;
//!                                            `notify` payload is
//!                                            `{ message, level }` table.
//! ```
//!
//! `ttymap.map:jump(...)` is fire-and-forget from the Lua side; the
//! matching `Receiver` on the App drains after each setup-state
//! callback. `ttymap.map:center()` reads a `Mutex<LonLat>` the
//! component refreshes at the start of every dispatch path that
//! carries a `Window` / `MapApi`, so callers see the latest centre
//! without threading anything through their signatures.
//!
//! `ttymap` is a Neovim-style **single-VM global**: `init.lua`
//! installs `ttymap.opt` / `ttymap.keymap` first; this module then
//! adds `ttymap.http` / `map` / `api` / `register_*` / `notify` /
//! `on_event` to the same table before plugins load. Every plugin
//! sees one unified `ttymap` namespace, and `init.lua` can
//! `require "ttymap.<plugin_name>"` and mutate a config table the
//! plugin will read later (Lua's module cache makes the require
//! return the same table on the plugin side).

pub mod http;
pub mod json;
pub mod map;
pub mod sgp4;

mod config;
mod help;
mod imperative;
mod log;
mod register;
mod storage;
mod tile;

use config::HostConfig;
use help::HostHelp;
use log::HostLog;
use map::HostMap;
use storage::HostStorage;
use tile::HostTile;

use std::sync::{Arc, Mutex};

use mlua::{Lua, Table};

use crate::host::{LuaHostHandles, LuaHostShared};
use ttymap_engine::geo::LonLat;
use ttymap_engine::shared::http::HttpClient;
use ttymap_shared::event::{Event, Level};
use ttymap_tui::compositor::op::Op;

// ── Install entry point ─────────────────────────────────────────────

/// Extend the `ttymap` global with the plugin runtime API
/// (`http` / `map` / `api` / `register_*` / `notify` / `on_event` …)
/// and return the host handles plugins read view state through.
///
/// The `ttymap` global must already exist — `init.lua`'s pre-pass
/// (see [`crate::init_lua`]) creates it with `opt` / `keymap`
/// before this runs. We add fields to the same table rather than
/// replace it, so `ttymap.opt.*` mutations from `init.lua` survive.
///
/// "Plugin" is purely a Lua-side concept — a `.lua` file's worth of
/// `register_palette_command` / `register_keybind` / `on_event`
/// calls. The host has no notion of plugin identity, no per-script
/// slot, no attribution. Each `register_*` call pushes directly into
/// `registry` and returns a handle to Lua.
pub fn install(
    lua: &Lua,
    shared: Arc<LuaHostShared>,
    ops: ttymap_tui::compositor::op::OpsBuffer,
    bus: std::rc::Rc<ttymap_shared::event::EventBus>,
    ticks: std::rc::Rc<crate::tick::TickRegistry>,
    registry: crate::registrar::LuaRegistryHandle,
    dirs: Option<&ttymap_config::AppDirs>,
) -> mlua::Result<LuaHostHandles> {
    // Fire-and-forget Lua intents (`map:jump`, `:zoom`, `:fly_to`,
    // `frame.export`) enqueue `Op::Command(UserCommand::...)` onto
    // `ops`; the App drains and dispatches per iteration alongside
    // every other source. Plugin trust model is nvim-style (anything
    // the user could do, a plugin can also do).
    let center = Arc::new(Mutex::new(LonLat { lon: 0.0, lat: 0.0 }));
    let zoom = Arc::new(Mutex::new(0.0_f64));

    let ttymap: Table = match lua.globals().get::<mlua::Value>("ttymap")? {
        mlua::Value::Table(t) => t,
        _ => {
            // No init pre-pass ran (test paths or future callers). Create
            // an empty `ttymap` so the rest of install can proceed.
            let t = lua.create_table()?;
            lua.globals().set("ttymap", t.clone())?;
            t
        }
    };

    // One row per `ttymap.<name>` userdata namespace — adding a domain
    // is one line here. `tests::USERDATA_NAMESPACES` mirrors the list
    // and the coverage test compares the two in both directions, so a
    // namespace added to only one of them fails loudly.
    let namespaces = [
        (
            "http",
            lua.create_userdata(http::HostHttp {
                http: HttpClient::new("lua").map_err(mlua::Error::external)?,
                cache_root: dirs.map(|d| d.cache.clone()),
            })?,
        ),
        (
            "map",
            lua.create_userdata(HostMap::new(ops.clone(), center.clone(), zoom.clone()))?,
        ),
        ("json", lua.create_userdata(json::HostJson)?),
        ("sgp4", lua.create_userdata(sgp4::HostSgp4)?),
        ("tile", lua.create_userdata(HostTile::new(shared.clone()))?),
        (
            "config",
            lua.create_userdata(HostConfig::new(shared.clone()))?,
        ),
        ("help", lua.create_userdata(HostHelp::new(shared.clone()))?),
        ("log", lua.create_userdata(HostLog::new("lua".to_string()))?),
    ];
    for (name, userdata) in namespaces {
        ttymap.set(name, userdata)?;
    }

    if let Some(host_storage) = HostStorage::new(dirs) {
        ttymap.set("storage", lua.create_userdata(host_storage)?)?;
    } else {
        // No per-user data dir resolved — `:open` would have nowhere
        // to write. Skip wiring rather than expose a userdata that
        // errors on every call; plugins probing `ttymap.storage` for
        // nil will see "feature unavailable" instead of cryptic
        // ProjectDirs failures.
        ::log::warn!("lua-host: ttymap.storage not installed (no data dir resolved)");
    }

    // Activation surfaces (`register_palette_command` /
    // `register_keybind` / `on_event`) — every call pushes directly
    // into the live `LuaRegistry` (or, for `on_event`, subscribes
    // directly against the bus / tick registry) and returns a
    // Lua-facing handle. No deferred capture, no per-script slot.
    register::install(lua, &ttymap, bus, ticks.clone(), registry, shared.clone())?;

    // Imperative primitives (`ttymap.api.{card,palette,frame}`) —
    // runtime-time `open` / `to_ansi` / `on_tick` calls a plugin
    // makes from inside its callbacks.
    imperative::install(lua, &ttymap, ops.clone(), shared.clone(), ticks)?;

    // ── ttymap.notify ────────────────────────────────────────────────
    //
    // Top-level write surface for transient status messages. Kept as
    // a plain function (not method-style) so callers write
    // `ttymap.notify("ok")` instead of `ttymap.notify:post("ok")` —
    // the call site is the common one. The notification rides the
    // shared `OpsBuffer` as `Op::Publish(Event::Notify {...})`; the
    // App-side dispatcher hands it to `EventBus::publish`, which
    // fans out to whoever subscribed (today: bundled `notify.lua`).
    let ops_for_notify = ops;
    ttymap.set(
        "notify",
        lua.create_function(move |_, (msg, opts): (String, Option<Table>)| {
            let level = opts
                .and_then(|t| t.get::<String>("level").ok())
                .map(|s| Level::parse(&s))
                .unwrap_or(Level::Info);
            ops_for_notify.borrow_mut().push(Op::Publish(Event::Notify {
                message: msg,
                level,
            }));
            Ok(())
        })?,
    )?;

    // ── ttymap.runtime_path ──────────────────────────────────────────
    //
    // The resolved runtime layer list as a 1-indexed Lua array.
    // Mostly informational on the Lua side today (vm::new_lua
    // already prepends `<layer>/lua/` to package.path for every
    // layer); kept exposed so future Lua libs that need to walk
    // the layer list (alternate searchers, lazy-loaders, etc.)
    // have it available without a Rust round-trip.
    let layers: Vec<String> = crate::runtime_path()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    ttymap.set("runtime_path", lua.create_sequence_from(layers)?)?;

    // ── ttymap.version ───────────────────────────────────────────────
    //
    // The workspace version (`version.workspace = true`), so plugins —
    // the bundled help popup today — can show it without a CLI round-trip.
    ttymap.set("version", env!("CARGO_PKG_VERSION"))?;

    Ok(LuaHostHandles { center, zoom })
}

#[cfg(test)]
mod tests;
