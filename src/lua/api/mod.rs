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
//! Submodules:
//! - [`sgp4`] — `ttymap.sgp4` userdata (TLE parsing + SGP4 propagation)
//! - [`map_api`] — per-frame `map` table built inside `Lua::scope`
//!   (drawing primitives that borrow the live ratatui buffer)
//!
//! Surface today:
//!
//! ```text
//! ttymap.http   :fetch(url) -> Job          background HTTP GET (UTF-8 body)
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
//! ttymap.sgp4   :parse_tle(text) -> handle  parse a TLE for SGP4 propagation
//! ttymap.sgp4   :parse_tles(text) -> array  parse a multi-TLE block (groups)
//! ttymap.sgp4   :propagate(h[, t]) -> table propagate a handle to unix time t
//! ttymap.sgp4   :propagate_batch(hs[, t])   batch propagate (Starlink-scale)
//! ttymap.tile   :attribution() -> string?   active tile provider's attribution
//! ttymap.config :geoip_endpoint() -> string `[geoip].endpoint` value
//! ttymap.help   :keymap_entries() -> list   built-in keymap rows for help
//! ttymap.help   :palette_entries() -> list  per-plugin metadata for help
//! ttymap.log    :info(msg) / :warn(msg) / :error(msg)
//!                                            forward to host log at
//!                                            target `lua[<plugin>]`
//! ttymap.api.card.open(spec) -> Handle    push a focused window
//!                                            (LuaCardComponent) onto
//!                                            the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.palette.open(spec) -> Handle   push a palette provider
//!                                            onto the stack; handle:close()
//!                                            pops it (idempotent)
//! ttymap.api.frame.export()                 snapshot the current frame to disk
//! ttymap.api.frame.on_tick(callback)        register a per-frame callback
//!                                            (called with `MapApi`); multiple
//!                                            calls per script are stacked
//! ttymap.notify(msg [, opts])               post a transient status message;
//!                                            opts.level is `info` (default) /
//!                                            `warn` / `error`. The bundled
//!                                            `notify` plugin renders recent
//!                                            entries in a corner.
//! ttymap.api.notify.recent(ttl_ms) -> list  active notifications (age < ttl)
//!                                            consumed by the bundled `notify`
//!                                            plugin's per-frame renderer
//! ```
//!
//! `ttymap.map:jump(...)` is fire-and-forget from the Lua side; the
//! matching `Receiver` on the App drains after each setup-state
//! callback. `ttymap.map:center()` reads a `Mutex<LonLat>` the
//! component refreshes at the start of every dispatch path that
//! carries a `Window` / `MapApi`, so callers see the latest centre
//! without threading anything through their signatures.
//!
//! Note: the same `ttymap` name is used by `init.lua` as a config DSL
//! (`ttymap.opt`, `ttymap.keymap`) — that's a different Lua state
//! (see `init_lua.rs`), so the namespaces don't collide at runtime.
//! The split is by *scope*, not by name: `opt` / `keymap` live in
//! init; `http` / `map` / etc. live in plugin runtime.

pub mod http;
pub mod json;
pub mod map_api;
pub mod sgp4;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use mlua::{Lua, Table, UserData};

use crate::app::UserIntent;
use crate::geo::LonLat;
use crate::map::MapAction;
use crate::shared::http::HttpClient;

/// Maximum number of pending notifications retained in the host's
/// shared ring buffer. Sized to absorb a brief flurry (a search
/// returning, a fetch erroring, a file exporting) without needing
/// per-call resizing — at typical 3-second display TTL the buffer
/// rarely hits cap, and on overflow we drop the oldest so newer
/// signals are never starved.
const NOTIFY_RING_CAP: usize = 16;

// ── Shared snapshot ─────────────────────────────────────────────────

/// Shared, mostly-immutable runtime data that every Lua plugin can
/// query via the `ttymap` global. Built once in [`crate::app::App::new`]
/// and Arc-cloned into each namespace userdata that reads from it.
///
/// Why not upvalue prepend? With ~10 builtin plugins each needing
/// different runtime data, prepending bespoke `local _X = [[...]]`
/// per plugin meant per-plugin Rust glue. A shared accessor surface
/// keeps the bridge uniform: bundled and user plugins both see the
/// same `ttymap.*` API, and adding a new builtin requires zero Rust.
pub struct LuaHostShared {
    /// Tile provider's attribution string. `None` when the active
    /// `TileClient` has no attribution to display (custom backends
    /// without OSM data, mostly).
    pub attribution: Option<String>,
    /// IP-geolocation endpoint URL (`[geoip].endpoint` in
    /// `config.toml`). The here plugin GETs this to resolve the
    /// user's coordinates.
    pub geoip_endpoint: String,
    /// Pre-baked `(key-binding, action-label)` pairs for built-in
    /// map actions. Help renders this as the keymap section of its
    /// cheatsheet. Built once at startup from the live `KeyMap` so
    /// runtime overrides surface correctly.
    pub keymap_entries: Vec<(String, String)>,
    /// Per-plugin metadata snapshot, appended during plugin
    /// registration. Held behind a `Mutex` so `LuaHostShared` can be
    /// Arc'd into each plugin's host namespaces at register time and
    /// populated later. Help reads this lazily (at render time, not
    /// register time) so it sees every plugin regardless of load
    /// order.
    pub palette_entries: Mutex<Vec<PluginEntry>>,
    /// Transient status messages posted via `ttymap.notify(msg, opts)`.
    /// The bundled `notify` plugin reads this each tick via
    /// `ttymap.api.notify.recent(ttl_ms)` and renders entries that are
    /// still within their TTL. A small ring (cap [`NOTIFY_RING_CAP`])
    /// — overflow drops the oldest. Plain `Vec` because the volume is
    /// tiny and the renderer iterates oldest-first by design.
    pub notifications: Mutex<VecDeque<NotifyEntry>>,
}

/// One transient status message awaiting display. Held in
/// [`LuaHostShared::notifications`]; the bundled `notify` plugin
/// surfaces these via `ttymap.api.notify.recent(ttl_ms)`. `level` is a
/// raw string ("info" / "warn" / "error") so renderers can map to
/// theme colours without the host pre-committing to a palette.
#[derive(Clone)]
pub struct NotifyEntry {
    pub message: String,
    pub level: String,
    pub posted_at: Instant,
}

/// One plugin's help-relevant metadata. Surfaced to Lua via
/// `ttymap.help:palette_entries()` so help.lua can render it without
/// caring about how the data was harvested. Only plugins with a
/// top-level keybinding land here; keyless plugins are filtered at
/// push time (matching the prior harvest's `!hint.is_empty()` rule).
#[derive(Clone)]
pub struct PluginEntry {
    pub name: String,
    pub key: String,
    pub label: String,
}

impl LuaHostShared {
    pub fn new(
        attribution: Option<String>,
        geoip_endpoint: String,
        keymap_entries: Vec<(String, String)>,
    ) -> Self {
        Self {
            attribution,
            geoip_endpoint,
            keymap_entries,
            palette_entries: Mutex::new(Vec::new()),
            notifications: Mutex::new(VecDeque::with_capacity(NOTIFY_RING_CAP)),
        }
    }

    /// Append one notification to the shared ring buffer. Oldest
    /// entry evicted on overflow so a flurry never starves the most
    /// recent signal. Poisoned mutex is silently skipped — losing a
    /// transient message is preferable to crashing the host.
    pub fn push_notification(&self, entry: NotifyEntry) {
        if let Ok(mut buf) = self.notifications.lock() {
            if buf.len() >= NOTIFY_RING_CAP {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }

    /// Append one plugin's metadata to the snapshot. Called once per
    /// plugin during registration. A poisoned mutex is silently
    /// skipped — losing a help row is preferable to crashing the host.
    pub fn push_palette_entry(&self, entry: PluginEntry) {
        if let Ok(mut slot) = self.palette_entries.lock() {
            slot.push(entry);
        }
    }

    /// All-empty default for tests and registration-time loads that
    /// don't need real runtime data. The `ttymap.*` host surface
    /// still installs in a Lua state used only to capture the
    /// script's `register_*` call.
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::new(None, String::new(), Vec::new()))
    }
}

// ── Per-component handles ───────────────────────────────────────────

/// Channels + shared state owned by **the setup state** (the Lua VM
/// that runs the script's top-level `register_*` calls and continues
/// to run palette / keybind callbacks for the program lifetime).
/// `install()` returns this once per state; the App routes the
/// shared cells to the right consumers.
///
/// - **UserIntent sender** (not part of these handles) — every
///   fire-and-forget Lua intent (`ttymap.map:jump` / `:zoom(level)` /
///   `:fly_to` / `ttymap.api.frame.export`) is pre-built into an
///   [`UserIntent`] on the Lua side and pushed through a `Sender<UserIntent>`
///   that every plugin clones from the **single** App-level channel.
///   The receiver lives directly on `App`; a single drain per frame
///   covers every plugin's intents.
/// - `center` / `zoom` — shared with `ttymap.map`'s userdata so
///   `ttymap.map:center()` and `:zoom()` return the live values.
///   Components refresh them on each dispatch path that carries a
///   `Window`.
///
/// Component pushes from `ttymap.api.card.open` /
/// `ttymap.api.palette.open` no longer live here — they ride the
/// shared [`OpsBuffer`](crate::compositor::op::OpsBuffer) as
/// [`Op::Push`](crate::compositor::op::Op::Push) and the App drains them
/// alongside [`Op::Close`](crate::compositor::op::Op::Close).
pub struct LuaHostHandles {
    pub center: Arc<Mutex<LonLat>>,
    /// Latest zoom level mirrored from the host so
    /// `ttymap.map:zoom()` (no-arg getter form) returns the current
    /// zoom from any callback context. Same Arc held by the
    /// [`HostMap`] userdata; refreshed on the dispatch paths that
    /// also refresh `center`.
    pub zoom: Arc<Mutex<f64>>,
}

// ── Self-registration capture ────────────────────────────────────────

/// One palette row declared by a plugin via
/// `ttymap.register_palette_command(spec)`. The `invoke` callback is
/// stored as a [`RegistryKey`] so it survives the registration call
/// and can be invoked from the persistent Lua state at activation
/// time. The state must be kept alive (held by the registrar) for
/// the program lifetime.
pub struct PaletteCommandSpec {
    pub label: String,
    pub hint: String,
    pub invoke: mlua::RegistryKey,
}

/// One keybind declared via `ttymap.register_keybind(key, callback)`.
/// `key` is a single Char activation; `callback` runs at press time
/// and (truthy return) opts into pushing the file's plugin component.
pub struct KeybindSpec {
    pub key: char,
    pub callback: mlua::RegistryKey,
}

/// One subscription declared via `ttymap.on_event(name, fn)` (or its
/// `ttymap.api.frame.on_tick(fn)` sugar, which lowers to event name
/// `"tick"`). The host walks these at register time and pushes one
/// [`Subscriber`](crate::lua::registry::Subscriber) into the
/// [`LuaEventBus`](crate::lua::LuaEventBus) bucket for `event_name`.
pub struct EventSubscription {
    pub event_name: &'static str,
    pub callback: mlua::RegistryKey,
}

/// Everything a single plugin file's setup phase declared. nvim-
/// style: each activation surface is a separate explicit call with
/// its own Lua callback. Plugins own whether/when to push by
/// inspecting their own state inside the callback and calling
/// `ttymap.api.card.open(spec)` / `ttymap.api.palette.open(spec)`.
/// Per-frame work subscribes via `ttymap.api.frame.on_tick(fn)` —
/// stacked: each call appends a callback that fires every frame.
/// Other events go through `ttymap.on_event(name, fn)`.
#[derive(Default)]
pub struct CapturedRegistration {
    /// Each `ttymap.register_palette_command({label, invoke})` call.
    pub palette_commands: Vec<PaletteCommandSpec>,
    /// Each `ttymap.register_keybind(key, callback)` call.
    pub keybinds: Vec<KeybindSpec>,
    /// Each `ttymap.on_event(name, fn)` call (and `on_tick` sugar).
    /// Order = registration order across event names.
    pub event_subscriptions: Vec<EventSubscription>,
}

/// Slot used by a fresh Lua state to capture the script's
/// registration calls. `Rc<RefCell<...>>` is fine — the Lua state
/// is single-threaded and the capture lifetime is bounded by
/// `lua.load(source).exec()`.
pub type CaptureSlot = Rc<RefCell<CapturedRegistration>>;

/// Build an empty capture slot. The caller (typically `fresh_load`)
/// passes one to [`install`] and reads it back after running the
/// script.
pub fn new_capture_slot() -> CaptureSlot {
    Rc::new(RefCell::new(CapturedRegistration::default()))
}

// ── Install entry point ─────────────────────────────────────────────

/// Build the `ttymap` table and install it as a Lua global. Returns
/// the channels the calling component drains after each callback. One
/// install per Lua state — same surface for components and palette
/// providers, so the bridge stays uniform.
///
/// `slot` receives any `register_palette_command` / `register_keybind`
/// declarations and any `ttymap.api.frame.on_tick` subscriptions the
/// script makes. Rust never inspects the script's return value or
/// table layout — the script is a plugin by virtue of existing in
/// `<runtime>/plugin/`, identity = file stem.
pub fn install(
    lua: &Lua,
    tag: &'static str,
    shared: Arc<LuaHostShared>,
    slot: CaptureSlot,
    ops: crate::compositor::op::OpsBuffer,
) -> mlua::Result<LuaHostHandles> {
    // Fire-and-forget Lua intents (`map:jump`, `:zoom`, `:fly_to`,
    // `frame.export`) enqueue `Op::Intent(UserIntent::...)` onto
    // `ops`; the App drains and dispatches per iteration alongside
    // every other source. Plugin trust model is nvim-style (anything
    // the user could do, a plugin can also do).
    let center = Arc::new(Mutex::new(LonLat { lon: 0.0, lat: 0.0 }));
    let zoom = Arc::new(Mutex::new(0.0_f64));

    let ttymap = lua.create_table()?;
    ttymap.set(
        "http",
        lua.create_userdata(http::HostHttp {
            http: HttpClient::new(tag),
        })?,
    )?;
    ttymap.set(
        "map",
        lua.create_userdata(HostMap {
            ops: ops.clone(),
            center: center.clone(),
            zoom: zoom.clone(),
        })?,
    )?;
    ttymap.set("json", lua.create_userdata(json::HostJson)?)?;
    ttymap.set("sgp4", lua.create_userdata(sgp4::HostSgp4)?)?;
    ttymap.set(
        "tile",
        lua.create_userdata(HostTile {
            shared: shared.clone(),
        })?,
    )?;
    ttymap.set(
        "config",
        lua.create_userdata(HostConfig {
            shared: shared.clone(),
        })?,
    )?;
    ttymap.set(
        "help",
        lua.create_userdata(HostHelp {
            shared: shared.clone(),
        })?,
    )?;
    ttymap.set(
        "log",
        lua.create_userdata(HostLog {
            target: format!("lua[{}]", tag),
        })?,
    )?;

    // Activation surfaces. Each is opt-in and explicit — the host
    // never auto-adds a palette row or keybind from the plugin's
    // `name` / `label` fields. The Lua callback (`spec.invoke` /
    // 2nd arg of register_keybind) is the plugin's chance to inspect
    // its own state and decide whether to push a fresh component:
    // truthy return → host pushes, falsy → no-op.
    let cap = slot.clone();
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
            cap.borrow_mut().palette_commands.push(PaletteCommandSpec {
                label,
                hint,
                invoke: invoke_key,
            });
            Ok(())
        })?,
    )?;
    let cap = slot.clone();
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
                cap.borrow_mut().keybinds.push(KeybindSpec {
                    key: c,
                    callback: callback_key,
                });
                Ok(())
            },
        )?,
    )?;

    // `ttymap.on_event(name, fn)` — generic pub/sub subscription.
    // Lower into a [`EventSubscription`] keyed by the leaked event
    // name; the host walks them at register time and pushes one
    // [`Subscriber`](crate::lua::registry::Subscriber) into the
    // matching [`LuaEventBus`](crate::lua::LuaEventBus) bucket.
    //
    // The leak is bounded by `(unique event names) × plugins`,
    // happens at register time only, and produces `&'static str`
    // (which the bus needs as a HashMap key matching plugin-name
    // and source-text leaks done elsewhere in `register_plugins_in`).
    //
    // `ttymap.api.frame.on_tick(fn)` is sugar for
    // `ttymap.on_event("tick", fn)` — same Subscriber shape, same
    // dispatch path, just a different surface for the common case.
    let cap = slot.clone();
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
                cap.borrow_mut()
                    .event_subscriptions
                    .push(EventSubscription {
                        event_name: leaked,
                        callback: key,
                    });
                Ok(())
            },
        )?,
    )?;
    // ── ttymap.api ────────────────────────────────────────────────
    //
    // The (nvim-style) plugin API surface. Currently hosts:
    //
    // - `ttymap.api.card.open(spec) -> CardHandle` — push a
    //   focused [`LuaCardComponent`] onto the compositor stack.
    // - `ttymap.api.palette.open(spec) -> PaletteHandle` — push a
    //   palette provider (a `PaletteComponent` wrapping a
    //   [`LuaPaletteProvider`]) onto the stack. Returning
    //   `{ switch = sub_spec }` from the provider's `execute` swaps
    //   the provider in place (sub-mode transition, no stacking).
    // - `ttymap.api.frame.export()` — request the current frame be
    //   snapshotted to disk.
    let api = lua.create_table()?;

    let card_api = lua.create_table()?;
    let ops_for_window = ops.clone();
    card_api.set(
        "open",
        lua.create_function(
            move |lua, spec: Table| -> mlua::Result<crate::lua::bridge::card_handle::CardHandle> {
                use crate::compositor::CardId;
                use crate::compositor::op::Op;
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
                // callbacks (`render`, `handle_event`, …) capture
                // upvalues in this state, so the per-window Lua handle
                // must be a clone of it (cheap Arc bump, no copy of the
                // VM). When `LuaCardComponent` later calls into those
                // callbacks, the same upvalue scope is in scope.
                let component = LuaCardComponent::from_spec(lua.clone(), spec, tag)?;
                ops_for_window.borrow_mut().push(Op::Push {
                    id,
                    component: Box::new(component) as Box<dyn crate::compositor::Component>,
                });
                Ok(CardHandle::new(id, ops_for_window.clone()))
            },
        )?,
    )?;
    api.set("card", card_api)?;

    // ── ttymap.api.palette ───────────────────────────────────────────
    //
    // Mirror of `ttymap.api.card.open`: build a palette provider on
    // the same Lua VM (the setup state), wrap it in a
    // [`PaletteComponent`], and enqueue an `Op::Push { id, component }`
    // onto the shared `OpsBuffer`. The App drains the buffer per
    // iteration and pushes via `compositor.push_with_id`, associating
    // the component with the [`CardId`] reserved here. The returned
    // [`PaletteHandle`] holds the same id and can request close via
    // [`Op::Close`].
    let palette_api = lua.create_table()?;
    let ops_for_palette = ops.clone();
    palette_api.set(
        "open",
        lua.create_function(
            move |lua,
                  spec: Table|
                  -> mlua::Result<crate::lua::bridge::palette_handle::PaletteHandle> {
                use crate::compositor::CardId;
                use crate::compositor::op::Op;
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
                ops_for_palette.borrow_mut().push(Op::Push {
                    id,
                    component: Box::new(palette) as Box<dyn crate::compositor::Component>,
                });
                Ok(PaletteHandle::new(id, ops_for_palette.clone()))
            },
        )?,
    )?;
    api.set("palette", palette_api)?;

    // ── ttymap.api.frame ─────────────────────────────────────────────
    //
    // Per-frame primitives:
    //
    // - `export()`     fire-and-forget request to snapshot the current
    //                  frame to disk; pushes `UserIntent::ExportFrame` onto
    //                  the shared `intent_tx`, drained per frame.
    // - `on_tick(fn)`  subscribe a callback to per-frame dispatch. The
    //                  host walks the captured registry keys after the
    //                  script runs and pushes one [`TickEntry`] per
    //                  call into the global `LuaTickRegistry`. Multiple
    //                  calls per script are stacked in registration
    //                  order.
    let frame_api = lua.create_table()?;
    let ops_for_export = ops.clone();
    frame_api.set(
        "export",
        lua.create_function(move |_, _: ()| {
            ops_for_export
                .borrow_mut()
                .push(crate::compositor::op::Op::Intent(UserIntent::ExportFrame));
            Ok(())
        })?,
    )?;
    // `on_tick` is a thin sugar for `on_event("tick", fn)` — kept
    // because the existing plugin set + docs use it everywhere and
    // it reads more naturally for the per-frame use case. New
    // event surfaces should use `ttymap.on_event` directly.
    let cap = slot.clone();
    frame_api.set(
        "on_tick",
        lua.create_function(move |lua, callback: mlua::Function| -> mlua::Result<()> {
            let key = lua.create_registry_value(callback)?;
            cap.borrow_mut()
                .event_subscriptions
                .push(EventSubscription {
                    event_name: "tick",
                    callback: key,
                });
            Ok(())
        })?,
    )?;
    api.set("frame", frame_api)?;

    // ── ttymap.api.notify ─────────────────────────────────────────────
    //
    // Read-side of the notification ring. The bundled `notify` plugin
    // walks `recent(ttl_ms)` per frame and renders entries whose age
    // is below the TTL. Returning age (rather than a wall-clock
    // timestamp) lets renderers decide on fade / sort policy without
    // owning a clock.
    let notify_api = lua.create_table()?;
    let shared_for_recent = shared.clone();
    notify_api.set(
        "recent",
        lua.create_function(move |lua, ttl_ms: u64| {
            let table = lua.create_table()?;
            let now = Instant::now();
            let buf = match shared_for_recent.notifications.lock() {
                Ok(g) => g,
                Err(_) => return Ok(table),
            };
            let mut idx = 1;
            for e in buf.iter() {
                let age_ms = now.saturating_duration_since(e.posted_at).as_millis() as u64;
                if age_ms < ttl_ms {
                    let row = lua.create_table()?;
                    row.set("message", e.message.as_str())?;
                    row.set("level", e.level.as_str())?;
                    row.set("age_ms", age_ms)?;
                    table.set(idx, row)?;
                    idx += 1;
                }
            }
            Ok(table)
        })?,
    )?;
    api.set("notify", notify_api)?;

    ttymap.set("api", api)?;

    // ── ttymap.notify ────────────────────────────────────────────────
    //
    // Top-level write surface for transient status messages. Kept as
    // a plain function (not method-style) so callers write
    // `ttymap.notify("ok")` instead of `ttymap.notify:post("ok")` —
    // the call site is the common one; the read side under
    // `ttymap.api.notify.recent` is consumed by exactly one plugin
    // (the bundled renderer).
    let shared_for_notify = shared.clone();
    ttymap.set(
        "notify",
        lua.create_function(move |_, (msg, opts): (String, Option<Table>)| {
            let level = opts
                .and_then(|t| t.get::<String>("level").ok())
                .unwrap_or_else(|| "info".to_string());
            shared_for_notify.push_notification(NotifyEntry {
                message: msg,
                level,
                posted_at: Instant::now(),
            });
            Ok(())
        })?,
    )?;

    lua.globals().set("ttymap", ttymap)?;

    Ok(LuaHostHandles { center, zoom })
}

// ── ttymap.map ────────────────────────────────────────────────────────

struct HostMap {
    /// Shared op buffer the lua subsystem drains every iteration.
    /// Fire-and-forget Lua intents (`jump` / `zoom` / `fly_to`)
    /// enqueue an `Op::Intent(UserIntent::Map(...))`; the host treats
    /// them identically to a keymap-driven dispatch.
    ops: crate::compositor::op::OpsBuffer,
    center: Arc<Mutex<LonLat>>,
    zoom: Arc<Mutex<f64>>,
}

impl UserData for HostMap {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.map:jump(lon, lat)` — request the map recentre on
        // the given coordinate. Enqueues `UserIntent::Map(Jump)` onto
        // the shared op buffer so the host treats it identically to a
        // keymap-driven jump.
        methods.add_method("jump", |_, this, (lon, lat): (f64, f64)| {
            this.ops
                .borrow_mut()
                .push(crate::compositor::op::Op::Intent(UserIntent::Map(
                    MapAction::Jump(LonLat { lon, lat }),
                )));
            Ok(())
        });

        // `ttymap.map:zoom([level])` — overloaded:
        //   `:zoom(level)` queues a zoom request (clamped host-side
        //   in `MapState::process_action`). Fire-and-forget.
        //   `:zoom()` (no arg) returns the current zoom level read
        //   from the shared `Arc<Mutex<f64>>` the host refreshes on
        //   the same dispatch paths it refreshes `:center()` on.
        // mlua dispatches by the supplied argument signature: nil →
        // `None` (getter), number → `Some(level)` (setter).
        methods.add_method("zoom", |_, this, level: Option<f64>| match level {
            Some(z) => {
                this.ops
                    .borrow_mut()
                    .push(crate::compositor::op::Op::Intent(UserIntent::Map(
                        MapAction::SetZoom(z),
                    )));
                Ok(mlua::Value::Nil)
            }
            None => {
                let z = *this.zoom.lock().expect("zoom mutex poisoned");
                Ok(mlua::Value::Number(z))
            }
        });

        // `ttymap.map:fly_to(lon, lat, zoom)` — composite recenter +
        // zoom in one dispatch. Emitting `jump` + `zoom` separately
        // would render two frames; this routes through `MapFlyTo`
        // so the user sees a single transition.
        methods.add_method("fly_to", |_, this, (lon, lat, zoom): (f64, f64, f64)| {
            this.ops
                .borrow_mut()
                .push(crate::compositor::op::Op::Intent(UserIntent::Map(
                    MapAction::FlyTo {
                        center: LonLat { lon, lat },
                        zoom,
                    },
                )));
            Ok(())
        });

        // `ttymap.map:center()` -> lon, lat — current map centre, kept
        // fresh by the host before each dispatch path that carries a
        // `Window` / `MapApi`. Plugins use this to scope upstream
        // queries (e.g. an OpenSky bounding box around the user's
        // view).
        methods.add_method("center", |_, this, _: ()| {
            let ll = *this.center.lock().expect("center mutex poisoned");
            Ok((ll.lon, ll.lat))
        });
    }
}

// ── ttymap.tile / ttymap.config / ttymap.help ─────────────────────────────

struct HostTile {
    shared: Arc<LuaHostShared>,
}

impl UserData for HostTile {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.tile:attribution() -> string | nil` — active
        // TileClient's attribution string (typically "© OpenStreetMap
        // …"). The attribution overlay paints this; other plugins may
        // use it for their own attribution rows.
        methods.add_method("attribution", |_, this, _: ()| {
            Ok(this.shared.attribution.clone())
        });
    }
}

struct HostConfig {
    shared: Arc<LuaHostShared>,
}

impl UserData for HostConfig {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.config:geoip_endpoint() -> string` — configured geoip
        // URL (`[geoip].endpoint` in config.toml). The here plugin
        // GETs this to resolve the user's location.
        methods.add_method("geoip_endpoint", |_, this, _: ()| {
            Ok(this.shared.geoip_endpoint.clone())
        });
    }
}

struct HostHelp {
    shared: Arc<LuaHostShared>,
}

impl UserData for HostHelp {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.help:keymap_entries() -> [{key, label}, …]` —
        // keybindings for built-in map actions, formatted for
        // help-style display. Always returns the same data
        // (immutable after startup).
        methods.add_method("keymap_entries", |lua, this, _: ()| {
            let table = lua.create_table()?;
            for (i, (key, label)) in this.shared.keymap_entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("key", key.as_str())?;
                row.set("label", label.as_str())?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });

        // `ttymap.help:palette_entries() -> [{name, key, label}, …]`
        // — snapshot of every plugin's metadata, appended during
        // registration. Read lazily so help can be loaded mid-
        // registration and still see every sibling at render time.
        // Returns an empty list when the snapshot hasn't been
        // populated yet.
        methods.add_method("palette_entries", |lua, this, _: ()| {
            let table = lua.create_table()?;
            let entries = this.shared.palette_entries.lock();
            let entries = match &entries {
                Ok(g) => g.as_slice(),
                Err(_) => &[],
            };
            for (i, entry) in entries.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("name", entry.name.as_str())?;
                row.set("key", entry.key.as_str())?;
                row.set("label", entry.label.as_str())?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });
    }
}

// ── ttymap.log ───────────────────────────────────────────────────────

/// Plugin-side logging sink. `target` is pre-formatted as
/// `lua[<plugin>]` so callers don't pay for the format on every line
/// and `RUST_LOG=lua[aircraft]=debug` filters cleanly. Mirrors the
/// host-side `log::warn!("lua[{tag}]: ...")` convention used elsewhere
/// in the bridge — same target shape, just opened up to scripts.
struct HostLog {
    target: String,
}

impl UserData for HostLog {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("info", |_, this, msg: String| {
            log::info!(target: &this.target, "{}", msg);
            Ok(())
        });
        methods.add_method("warn", |_, this, msg: String| {
            log::warn!(target: &this.target, "{}", msg);
            Ok(())
        });
        methods.add_method("error", |_, this, msg: String| {
            log::error!(target: &this.target, "{}", msg);
            Ok(())
        });
    }
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper for tests: install the `ttymap` table into a fresh Lua
    /// and hand back the host handles + the shared op buffer. Mirrors
    /// the production install path; the capture slot is dropped since
    /// these tests don't exercise registration.
    fn install_for_test() -> (mlua::Lua, LuaHostHandles, crate::compositor::op::OpsBuffer) {
        let lua = mlua::Lua::new();
        let slot = new_capture_slot();
        let ops = crate::compositor::op::new_ops_buffer();
        let handles = install(&lua, "lua-test", LuaHostShared::empty(), slot, ops.clone())
            .expect("install ttymap table");
        (lua, handles, ops)
    }

    #[test]
    fn ttymap_table_is_installed_with_namespaces() {
        let (lua, _handles, _ops) = install_for_test();
        // Each namespace lookup must return a userdata; the shape
        // confirms the install wired all namespaces in.
        for ns in [
            "http", "map", "json", "sgp4", "tile", "config", "help", "log",
        ] {
            let ud: mlua::AnyUserData = lua
                .load(format!("return ttymap.{ns}"))
                .eval()
                .unwrap_or_else(|e| panic!("ttymap.{ns} should be a userdata: {e}"));
            // Just confirm round-trip works.
            let _ = ud;
        }
    }

    #[test]
    fn host_map_jump_pushes_appmsg_jump() {
        // `ttymap.map:jump(lon, lat)` enqueues a fully-formed
        // `Op::Intent(UserIntent::Map(MapAction::Jump(LonLat)))` on
        // the shared op buffer; the App drains and dispatches.
        let (lua, _handles, ops) = install_for_test();

        // Lua-side call: longitude first, then latitude.
        lua.load("ttymap.map:jump(139.7595, 35.6828)")
            .exec()
            .expect("exec");

        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Intent(UserIntent::Map(MapAction::Jump(ll))) => {
                assert!((ll.lon - 139.7595).abs() < 1e-9);
                assert!((ll.lat - 35.6828).abs() < 1e-9);
            }
            other => panic!("expected Op::Intent(Map(Jump)), got {other:?}"),
        }
    }

    #[test]
    fn host_map_zoom_setter_pushes_appmsg_set_zoom() {
        // `ttymap.map:zoom(level)` is fire-and-forget on the Lua side —
        // the level lands on the op buffer as
        // `Op::Intent(UserIntent::Map(MapAction::SetZoom(level)))`.
        let (lua, _handles, ops) = install_for_test();
        lua.load("ttymap.map:zoom(7.5)").exec().expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Intent(UserIntent::Map(MapAction::SetZoom(z))) => {
                assert!((z - 7.5).abs() < 1e-9)
            }
            other => panic!("expected Op::Intent(Map(SetZoom)), got {other:?}"),
        }
    }

    #[test]
    fn host_map_zoom_getter_reads_shared_cell() {
        // `ttymap.map:zoom()` (no args) reads the host-mirrored zoom
        // cell. The host writes via the same Arc the userdata holds,
        // so simulate a dispatch refresh by writing to `handles.zoom`
        // directly and assert Lua sees the new value. Symmetric with
        // the `:center()` pattern. Confirms (a) no-arg call doesn't
        // accidentally fall through to the setter and (b) the value
        // round-trips as a Lua number.
        let (lua, handles, ops) = install_for_test();
        *handles.zoom.lock().unwrap() = 9.25;
        let z: f64 = lua.load("return ttymap.map:zoom()").eval().expect("eval");
        assert!((z - 9.25).abs() < 1e-9);
        // Calling the getter must not enqueue a setter request.
        assert!(ops.borrow().is_empty());
    }

    #[test]
    fn host_map_fly_to_pushes_appmsg_fly_to() {
        // `ttymap.map:fly_to(lon, lat, zoom)` packs both into a single
        // `Op::Intent(UserIntent::Map(MapAction::FlyTo))` so the host
        // emits one dispatch per call (single redraw, no intermediate
        // frame).
        let (lua, _handles, ops) = install_for_test();
        lua.load("ttymap.map:fly_to(139.7595, 35.6828, 12.0)")
            .exec()
            .expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            crate::compositor::op::Op::Intent(UserIntent::Map(MapAction::FlyTo {
                center,
                zoom,
            })) => {
                assert!((center.lon - 139.7595).abs() < 1e-9);
                assert!((center.lat - 35.6828).abs() < 1e-9);
                assert!((zoom - 12.0).abs() < 1e-9);
            }
            other => panic!("expected Op::Intent(Map(FlyTo)), got {other:?}"),
        }
    }

    #[test]
    fn api_frame_export_pushes_appmsg_export_frame() {
        let (lua, _handles, ops) = install_for_test();
        lua.load("ttymap.api.frame.export()").exec().expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1);
        assert!(
            matches!(
                &drained[0],
                crate::compositor::op::Op::Intent(UserIntent::ExportFrame)
            ),
            "got {:?}",
            drained[0]
        );
    }

    #[test]
    fn api_frame_on_tick_registers_each_callback() {
        // Each `ttymap.api.frame.on_tick(fn)` call must land as one
        // entry in the capture slot's `ticks` vector. Multiple calls
        // stack in registration order; the host walks the slot at
        // `register_one` time and pushes one TickEntry per key.
        let lua = mlua::Lua::new();
        let slot = new_capture_slot();
        let _handles = install(
            &lua,
            "lua-test",
            LuaHostShared::empty(),
            slot.clone(),
            crate::compositor::op::new_ops_buffer(),
        )
        .expect("install ttymap table");
        lua.load(
            r#"
            ttymap.api.frame.on_tick(function() end)
            ttymap.api.frame.on_tick(function() end)
            "#,
        )
        .exec()
        .expect("exec");
        let cap = slot.borrow();
        assert_eq!(
            cap.event_subscriptions.len(),
            2,
            "two on_tick calls -> two entries"
        );
        assert!(
            cap.event_subscriptions
                .iter()
                .all(|s| s.event_name == "tick"),
            "on_tick must lower to event name `tick`"
        );
    }

    #[test]
    fn on_event_captures_subscription_under_the_named_event() {
        // `ttymap.on_event(name, fn)` — generic surface. Each call
        // must land as one entry tagged with the supplied event name.
        // Multiple distinct names lower to distinct buckets in the
        // event bus.
        let lua = mlua::Lua::new();
        let slot = new_capture_slot();
        let _handles = install(
            &lua,
            "lua-test",
            LuaHostShared::empty(),
            slot.clone(),
            crate::compositor::op::new_ops_buffer(),
        )
        .expect("install ttymap table");
        lua.load(
            r#"
            ttymap.on_event("tick", function() end)
            ttymap.on_event("frame_ready", function() end)
            ttymap.on_event("frame_ready", function() end)
            "#,
        )
        .exec()
        .expect("exec");
        let cap = slot.borrow();
        assert_eq!(cap.event_subscriptions.len(), 3);
        let names: Vec<&str> = cap
            .event_subscriptions
            .iter()
            .map(|s| s.event_name)
            .collect();
        assert_eq!(names, vec!["tick", "frame_ready", "frame_ready"]);
    }

    #[test]
    fn on_event_rejects_empty_name() {
        // Empty event names would land in a HashMap bucket that's
        // unreachable from any sensible dispatch call — surface an
        // error at register time so the plugin author finds it.
        let lua = mlua::Lua::new();
        let slot = new_capture_slot();
        let _handles = install(
            &lua,
            "lua-test",
            LuaHostShared::empty(),
            slot,
            crate::compositor::op::new_ops_buffer(),
        )
        .expect("install ttymap table");
        let result: mlua::Result<()> = lua.load(r#"ttymap.on_event("", function() end)"#).exec();
        assert!(result.is_err(), "empty event name should error");
    }

    #[test]
    fn url_encode_round_trips_query_chars() {
        let (lua, _handles, _ops) = install_for_test();
        // Spaces become `+`, reserved chars become `%HH`, unicode is
        // percent-encoded byte by byte.
        let encoded: String = lua
            .load(r#"return ttymap.http:url_encode("São Paulo?")"#)
            .eval()
            .expect("eval");
        assert_eq!(encoded, "S%C3%A3o+Paulo%3F");
        let plain: String = lua
            .load(r#"return ttymap.http:url_encode("abc-_.~")"#)
            .eval()
            .expect("eval");
        assert_eq!(plain, "abc-_.~");
    }

    #[test]
    fn parse_json_round_trips_primitives() {
        let (lua, _handles, _ops) = install_for_test();
        let n: i64 = lua
            .load(r#"return ttymap.json:parse("42")"#)
            .eval()
            .expect("eval");
        assert_eq!(n, 42);
        let s: String = lua
            .load(r#"return ttymap.json:parse('"hi"')"#)
            .eval()
            .expect("eval");
        assert_eq!(s, "hi");
        let b: bool = lua
            .load(r#"return ttymap.json:parse("true")"#)
            .eval()
            .expect("eval");
        assert!(b);
    }

    #[test]
    fn parse_json_object_becomes_string_keyed_table() {
        let (lua, _handles, _ops) = install_for_test();
        let (name, age): (String, i64) = lua
            .load(
                r#"
                local t = ttymap.json:parse('{"name": "alice", "age": 30}')
                return t.name, t.age
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(name, "alice");
        assert_eq!(age, 30);
    }

    #[test]
    fn parse_json_array_is_one_indexed_in_lua() {
        let (lua, _handles, _ops) = install_for_test();
        // Lua arrays are 1-indexed; t[1] is the first element.
        let (first, third, len): (i64, i64, i64) = lua
            .load(
                r#"
                local t = ttymap.json:parse("[10, 20, 30]")
                return t[1], t[3], #t
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(first, 10);
        assert_eq!(third, 30);
        assert_eq!(len, 3);
    }

    #[test]
    fn parse_json_invalid_returns_nil() {
        let (lua, _handles, _ops) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("not json !")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn parse_json_null_is_nil() {
        let (lua, _handles, _ops) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("null")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn notify_writes_into_shared_ring_and_recent_filters_by_ttl() {
        // `ttymap.notify(msg, opts)` writes a [`NotifyEntry`] into the
        // shared ring; `ttymap.api.notify.recent(ttl_ms)` returns the
        // currently-active subset so the bundled plugin can render.
        // Default level is "info"; explicit `level = "warn" / "error"`
        // round-trips. A long ttl shows everything; a zero ttl hides
        // everything (nothing has age < 0ms).
        let lua = mlua::Lua::new();
        let shared = LuaHostShared::empty();
        let slot = new_capture_slot();
        let _handles = install(
            &lua,
            "lua-test",
            shared.clone(),
            slot,
            crate::compositor::op::new_ops_buffer(),
        )
        .expect("install ttymap table");

        lua.load(
            r#"
            ttymap.notify("ok")
            ttymap.notify("watch out", { level = "warn" })
            ttymap.notify("boom", { level = "error" })
            "#,
        )
        .exec()
        .expect("notify writes");

        // Direct buffer inspection — independent of the recent()
        // surface so the test still pinpoints whichever side is wrong.
        {
            let buf = shared.notifications.lock().expect("lock");
            assert_eq!(buf.len(), 3, "three notify calls -> three entries");
            assert_eq!(buf[0].level, "info");
            assert_eq!(buf[1].level, "warn");
            assert_eq!(buf[2].level, "error");
            assert_eq!(buf[2].message, "boom");
        }

        // recent() with a generous ttl returns oldest-first; with
        // zero ttl returns nothing (every entry has age >= 0).
        let visible: i64 = lua
            .load(r#"return #ttymap.api.notify.recent(60000)"#)
            .eval()
            .expect("recent(60000)");
        assert_eq!(visible, 3);
        let none: i64 = lua
            .load(r#"return #ttymap.api.notify.recent(0)"#)
            .eval()
            .expect("recent(0)");
        assert_eq!(none, 0);
    }

    #[test]
    fn notify_ring_evicts_oldest_on_overflow() {
        // Past `NOTIFY_RING_CAP` writes the oldest entry must be
        // dropped so a flurry never strands the most recent signal
        // behind stale ones. Asserts the eviction happens by checking
        // that the head shifts forward, not that the cap is honoured
        // (cap is an internal invariant; what plugins observe is
        // "newest always wins").
        let lua = mlua::Lua::new();
        let shared = LuaHostShared::empty();
        let slot = new_capture_slot();
        let _handles = install(
            &lua,
            "lua-test",
            shared.clone(),
            slot,
            crate::compositor::op::new_ops_buffer(),
        )
        .expect("install ttymap table");
        for i in 0..(NOTIFY_RING_CAP + 4) {
            lua.load(format!(r#"ttymap.notify("msg-{}")"#, i))
                .exec()
                .expect("exec");
        }
        let buf = shared.notifications.lock().expect("lock");
        assert_eq!(
            buf.len(),
            NOTIFY_RING_CAP,
            "buffer must cap at {NOTIFY_RING_CAP}"
        );
        assert_eq!(
            buf.front().expect("non-empty").message,
            format!("msg-{}", 4),
            "first 4 entries should have been evicted"
        );
    }

    #[test]
    fn log_namespace_methods_round_trip() {
        // `ttymap.log:info/warn/error` are thin wrappers — no return
        // value, no error path. The unit test confirms the bindings
        // exist and accept a string. Anything observable downstream
        // (target = "lua[<plugin>]") is exercised by integration; here
        // we just want a panic-free round trip.
        let (lua, _handles, _ops) = install_for_test();
        lua.load(
            r#"
            ttymap.log:info("info-ok")
            ttymap.log:warn("warn-ok")
            ttymap.log:error("error-ok")
            "#,
        )
        .exec()
        .expect("log methods must round-trip");
    }

    #[test]
    fn sgp4_namespace_propagates_iss_through_lua() {
        // End-to-end: a Lua script calls parse_tle + propagate and
        // gets a position table back. Catches bridge wiring bugs
        // (userdata borrow, namespace install, table return shape)
        // that the standalone sgp4 module tests miss.
        let (lua, _handles, _ops) = install_for_test();
        let pos: mlua::Table = lua
            .load(
                r#"
                local tle = ttymap.sgp4:parse_tle(
                    "ISS (ZARYA)\n" ..
                    "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927\n" ..
                    "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537"
                )
                return ttymap.sgp4:propagate(tle, 1220568000)
                "#,
            )
            .eval()
            .expect("propagate from Lua");
        let lon: f64 = pos.get("lon").expect("lon");
        let lat: f64 = pos.get("lat").expect("lat");
        let alt: f64 = pos.get("alt_km").expect("alt_km");
        let vel: f64 = pos.get("vel_kms").expect("vel_kms");
        assert!((-180.0..=180.0).contains(&lon));
        assert!((-90.0..=90.0).contains(&lat));
        assert!(
            (300.0..500.0).contains(&alt),
            "altitude {alt} km not LEO-ish",
        );
        assert!((7.0..8.0).contains(&vel), "velocity {vel} not ISS-ish");
    }

    #[test]
    fn api_card_open_pushes_component_and_returns_handle() {
        // `ttymap.api.card.open(spec)` must do two things on the same
        // call: enqueue an `Op::Push` onto the shared `OpsBuffer` so
        // the App can push the component onto the compositor stack,
        // and hand back a `CardHandle` whose `:close()` enqueues
        // `Op::Close` keyed by the same id. Both behaviours are
        // independent of any `App` plumbing — this is the unit-level
        // proof that the primitive itself is wired right.
        let (lua, _handles, ops) = install_for_test();
        lua.load(
            r#"
            local h = ttymap.api.card.open({
                name = "demo",
                layout = { anchor = "left", width = 30 },
                render = function() return { "hello" } end,
            })
            ttymap_test_handle = h
            "#,
        )
        .exec()
        .expect("exec");
        // Exactly one Op::Push must be enqueued — `card.open` pushes
        // per call, no implicit dedup.
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one card.open -> one Op");
        let push_id = match &drained[0] {
            crate::compositor::op::Op::Push { id, .. } => *id,
            other => panic!("expected Op::Push, got {:?}", other),
        };
        // Close the handle from Lua — must enqueue Op::Close keyed by
        // the same id reserved at the call site.
        lua.load("ttymap_test_handle:close()")
            .exec()
            .expect("close");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one close() -> one Op");
        match &drained[0] {
            crate::compositor::op::Op::Close(id) => assert_eq!(*id, push_id),
            other => panic!("expected Op::Close, got {:?}", other),
        }
    }

    #[test]
    fn api_palette_open_pushes_component_and_returns_handle() {
        // Mirror of `api_card_open_pushes_component_and_returns_handle`:
        // `ttymap.api.palette.open(spec)` must enqueue an `Op::Push`
        // for the wrapped `PaletteComponent` and hand back a
        // `PaletteHandle` whose `:close()` enqueues `Op::Close` keyed
        // by the same id — no `App` plumbing required.
        let (lua, _handles, ops) = install_for_test();
        lua.load(
            r#"
            local h = ttymap.api.palette.open({
                prompt = "/",
                filter = function(_) end,
                items = function() return {} end,
                execute = function(_) return { close = true } end,
                is_loading = function() return false end,
            })
            ttymap_test_palette = h
            "#,
        )
        .exec()
        .expect("exec");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 1, "one palette.open -> one Op");
        let push_id = match &drained[0] {
            crate::compositor::op::Op::Push { id, .. } => *id,
            other => panic!("expected Op::Push, got {:?}", other),
        };
        // `:close()` is idempotent: each call enqueues an Op::Close —
        // close_by_id treats the second one as a no-op once the
        // component is already off the stack.
        lua.load("ttymap_test_palette:close(); ttymap_test_palette:close()")
            .exec()
            .expect("close");
        let drained: Vec<crate::compositor::op::Op> = std::mem::take(&mut *ops.borrow_mut());
        assert_eq!(drained.len(), 2, "two close() -> two Op::Close");
        for op in drained {
            match op {
                crate::compositor::op::Op::Close(id) => assert_eq!(id, push_id),
                other => panic!("expected Op::Close, got {:?}", other),
            }
        }
    }
}
