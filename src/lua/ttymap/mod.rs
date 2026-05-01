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
//! ttymap.map    :center() -> lon, lat       latest centre, refreshed per dispatch
//! ttymap.window :close()                    pop this component off the stack
//! ttymap.window :export_frame()             snapshot the current frame to disk
//! ttymap.json   :parse(s) -> value|nil      JSON → Lua tables (errors → nil)
//! ttymap.sgp4   :parse_tle(text) -> handle  parse a TLE for SGP4 propagation
//! ttymap.sgp4   :parse_tles(text) -> array  parse a multi-TLE block (groups)
//! ttymap.sgp4   :propagate(h[, t]) -> table propagate a handle to unix time t
//! ttymap.sgp4   :propagate_batch(hs[, t])   batch propagate (Starlink-scale)
//! ttymap.tile   :attribution() -> string?   active tile provider's attribution
//! ttymap.config :geoip_endpoint() -> string `[geoip].endpoint` value
//! ttymap.help   :keymap_entries() -> list   built-in keymap rows for help
//! ttymap.help   :palette_entries() -> list  per-plugin metadata for help
//! ttymap.plugin :open()                     ask the host to push a fresh
//!                                            instance of this plugin onto
//!                                            the compositor stack
//! ttymap.plugin :close()                    ask the host to close the
//!                                            currently-open instance of
//!                                            this plugin (no-op if none)
//! ```
//!
//! `ttymap.plugin` is exposed only on a script's **setup state** — the
//! Lua state that ran the top-level `ttymap.register_*` calls and
//! continues to run palette / keybind callbacks for the program
//! lifetime. Per-instance Lua states (the ones backing each pushed
//! `LuaComponent`) don't get this userdata; an instance closes itself
//! via `ttymap.window:close()`.
//!
//! `ttymap.map:jump(...)` and `ttymap.window:close()` are
//! fire-and-forget from the Lua side; the matching `Receiver`s on
//! [`LuaComponent`] drain after each callback while the `Window` is
//! still in scope. `ttymap.map:center()` reads a `Mutex<LonLat>` the
//! component refreshes at the start of every dispatch path that
//! carries a `Window` / `MapApi`, so callers see the latest centre
//! without threading anything through their signatures.
//!
//! Note: the same `ttymap` name is used by `init.lua` as a config DSL
//! (`ttymap.opt`, `ttymap.keymap`) — that's a different Lua state
//! (see `init_lua.rs`), so the namespaces don't collide at runtime.
//! The split is by *scope*, not by name: `opt` / `keymap` live in
//! init; `http` / `map` / `window` / etc. live in plugin runtime.

pub mod map_api;
pub mod sgp4;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::SystemTime;

use mlua::{Lua, Table, UserData};

use crate::geo::LonLat;
use crate::shared::http::HttpClient;

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
    /// Plugin-local keybindings parsed from `module.footer_hints`.
    /// Empty when the script omits the field.
    pub footer_hints: Vec<(String, String)>,
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

/// Channels + shared state the `LuaComponent` needs to drive the host
/// namespaces from outside Lua. Built once per host install and
/// threaded into the component at construction.
pub struct LuaHostHandles {
    pub jump_rx: mpsc::Receiver<LonLat>,
    pub close_rx: mpsc::Receiver<()>,
    pub export_rx: mpsc::Receiver<()>,
    pub center: Arc<Mutex<LonLat>>,
}

/// Per-plugin-file open/close primitives, exposed to Lua via
/// `ttymap.plugin:open()` and `ttymap.plugin:close()`. Setup callbacks
/// (palette command / keybind invoke) flip these atomics; Rust
/// drains them: `open` after the callback returns (push a fresh
/// component) and `close` on the next [`Component::poll`] tick of
/// the [`InstanceGuard`] wrapping the plugin's component.
#[derive(Clone, Default)]
pub struct PluginCtl {
    pub open_request: Arc<std::sync::atomic::AtomicBool>,
    pub close_request: Arc<std::sync::atomic::AtomicBool>,
}

// ── Self-registration capture ────────────────────────────────────────

/// What a plugin script declared by calling `register_plugin /
/// register_palette / register_overlay`. Exactly one of these per
/// script — the kind determines what factory the activation
/// surfaces (`palette_commands` / `keybinds`) push.
pub enum CapturedKind {
    /// Stack-pushed Component plugin (rendered panel).
    Plugin(Table),
    /// Palette provider plugin (`/`-style picker).
    Palette(Table),
    /// Always-on overlay (paints every frame, no focus, no key).
    /// Has no activation surface — `palette_commands` / `keybinds`
    /// are silently ignored when the script registers an overlay.
    Overlay(Table),
}

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

/// Everything a single plugin file's setup phase declared. nvim-
/// style explicit opt-in: the Component itself is one call
/// (`register_plugin`/`register_palette`/`register_overlay`),
/// and each activation surface (palette row, keybind) is a
/// **separate** explicit call with its own Lua callback. Plugins
/// own whether/when to push by inspecting their own state inside
/// the callback and returning truthy or falsy.
#[derive(Default)]
pub struct CapturedRegistration {
    /// The plugin kind itself. `None` means the script never called
    /// any of `register_plugin / register_palette / register_overlay`
    /// — the walker logs + skips that file.
    pub kind: Option<CapturedKind>,
    /// Each `ttymap.register_palette_command({label, invoke})` call.
    pub palette_commands: Vec<PaletteCommandSpec>,
    /// Each `ttymap.register_keybind(key, callback)` call.
    pub keybinds: Vec<KeybindSpec>,
    /// Each `ttymap.register_footer_hint({ key, label })` call,
    /// surfaced via `ttymap.help:palette_entries()` so the help
    /// cheatsheet shows in-panel keys per plugin. Empty by default.
    pub footer_hints: Vec<(String, String)>,
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
/// `slot` receives the spec from a `ttymap.register_plugin(...)` /
/// `ttymap.register_palette(...)` call inside the script. The Lua
/// subsystem walks plugin files and runs each script; the script
/// itself decides whether (and how) to register. Rust never inspects
/// the script's return value or table layout.
pub fn install(
    lua: &Lua,
    tag: &'static str,
    shared: Arc<LuaHostShared>,
    slot: CaptureSlot,
    plugin_ctl: Option<PluginCtl>,
) -> mlua::Result<LuaHostHandles> {
    let (jump_tx, jump_rx) = mpsc::channel();
    let (close_tx, close_rx) = mpsc::channel();
    let (export_tx, export_rx) = mpsc::channel();
    let center = Arc::new(Mutex::new(LonLat { lon: 0.0, lat: 0.0 }));

    let ttymap = lua.create_table()?;
    ttymap.set(
        "http",
        lua.create_userdata(HostHttp {
            http: HttpClient::new(tag),
        })?,
    )?;
    ttymap.set(
        "map",
        lua.create_userdata(HostMap {
            jump_tx,
            center: center.clone(),
        })?,
    )?;
    ttymap.set(
        "window",
        lua.create_userdata(HostWindow {
            close_tx,
            export_tx,
        })?,
    )?;
    ttymap.set("json", lua.create_userdata(HostJson)?)?;
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
    ttymap.set("help", lua.create_userdata(HostHelp { shared })?)?;

    // `ttymap.plugin` exists only on setup states (those with a
    // `plugin_ctl`) — it lets a setup callback signal Rust to
    // push a fresh instance or close the currently-open one. The
    // per-instance Component states don't get this userdata; they
    // close themselves via `ttymap.window:close()`.
    if let Some(ctl) = plugin_ctl {
        ttymap.set("plugin", lua.create_userdata(HostPlugin { ctl })?)?;
    }

    // Self-registration entry points. The script calls one of these
    // (at most once per script) to declare itself. The Lua subsystem
    // doesn't know what's a plugin or what kind it is until this
    // call lands. A double-call (or a mix of plugin + palette in
    // the same file) is a Lua-side error — surfaced via mlua so the
    // walker logs + skips the script.
    // Register the Component itself. Exactly one of these three
    // per script. A second call on any of them is a Lua-side error.
    fn set_kind(slot: &CaptureSlot, kind: CapturedKind, who: &str) -> mlua::Result<()> {
        let mut cap = slot.borrow_mut();
        if cap.kind.is_some() {
            return Err(mlua::Error::external(format!(
                "ttymap.{}: a plugin/palette/overlay was already registered in this script",
                who
            )));
        }
        cap.kind = Some(kind);
        Ok(())
    }
    let cap = slot.clone();
    ttymap.set(
        "register_plugin",
        lua.create_function(move |_, spec: Table| {
            set_kind(&cap, CapturedKind::Plugin(spec), "register_plugin")
        })?,
    )?;
    let cap = slot.clone();
    ttymap.set(
        "register_palette",
        lua.create_function(move |_, spec: Table| {
            set_kind(&cap, CapturedKind::Palette(spec), "register_palette")
        })?,
    )?;
    let cap = slot.clone();
    ttymap.set(
        "register_overlay",
        lua.create_function(move |_, spec: Table| {
            set_kind(&cap, CapturedKind::Overlay(spec), "register_overlay")
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
    let cap = slot;
    ttymap.set(
        "register_footer_hint",
        lua.create_function(move |_, spec: Table| -> mlua::Result<()> {
            let key: String = spec.get("key").or_else(|_| spec.get(1)).unwrap_or_default();
            let label: String = spec
                .get("label")
                .or_else(|_| spec.get(2))
                .unwrap_or_default();
            if !key.is_empty() || !label.is_empty() {
                cap.borrow_mut().footer_hints.push((key, label));
            }
            Ok(())
        })?,
    )?;

    lua.globals().set("ttymap", ttymap)?;

    Ok(LuaHostHandles {
        jump_rx,
        close_rx,
        export_rx,
        center,
    })
}

// ── ttymap.http ───────────────────────────────────────────────────────

struct HostHttp {
    http: HttpClient,
}

impl UserData for HostHttp {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.http:fetch(url)` — spawn a background GET and return a
        // Job. Body is decoded as UTF-8; non-text or fetch errors
        // surface as the Job never producing a result (try_take keeps
        // returning nil).
        methods.add_method("fetch", |_, this, url: String| {
            Ok(LuaJob::spawn(&this.http, url, None))
        });

        // `ttymap.http:fetch_cached(url, ttl_secs)` — disk-cached GET.
        // Read-through: on a fresh-enough cache hit (`age < ttl_secs`)
        // emits the cached body without touching the network. On miss,
        // does a real fetch and write-throughs the response. On HTTP
        // error, falls back to the stale on-disk copy if one exists —
        // critical for upstreams like CelesTrak's `gp.php`, which 403s
        // a same-IP repeat fetch within its own 2h refresh window and
        // would otherwise strand the plugin on "awaiting" forever.
        // Cache lives under `<XDG_CACHE_HOME>/ttymap/lua-http/` keyed
        // by FNV-1a of the URL.
        methods.add_method("fetch_cached", |_, this, (url, ttl_secs): (String, u64)| {
            Ok(LuaJob::spawn(&this.http, url, Some(ttl_secs)))
        });

        // `ttymap.http:url_encode(s)` — percent-encode a query string per
        // RFC 3986: unreserved (A-Za-z0-9-_.~) pass through, space
        // becomes `+`, everything else `%HH`. Lives here because most
        // callers urlencode arguments before handing them to `fetch`.
        methods.add_method("url_encode", |_, _this, s: String| {
            Ok(crate::shared::http::url::urlencoded(&s))
        });
    }
}

// ── ttymap.map ────────────────────────────────────────────────────────

struct HostMap {
    jump_tx: mpsc::Sender<LonLat>,
    center: Arc<Mutex<LonLat>>,
}

impl UserData for HostMap {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.map:jump(lon, lat)` — request the map recentre on the
        // given coordinate. The actual `AppMsg::Map(Action::Jump)`
        // emit happens when the matching `LuaComponent` drains the
        // channel after its current callback returns, so this is
        // fire-and-forget from the Lua side. Send errors (channel
        // disconnected) mean the component is being torn down —
        // silently ignore.
        methods.add_method("jump", |_, this, (lon, lat): (f64, f64)| {
            let _ = this.jump_tx.send(LonLat { lon, lat });
            Ok(())
        });

        // `ttymap.map:center()` -> lon, lat — current map centre, kept
        // fresh by the LuaComponent before each dispatch. Plugins
        // use this to scope upstream queries (e.g. an OpenSky
        // bounding box around the user's view).
        methods.add_method("center", |_, this, _: ()| {
            let ll = *this.center.lock().expect("center mutex poisoned");
            Ok((ll.lon, ll.lat))
        });
    }
}

// ── ttymap.plugin ─────────────────────────────────────────────────────

/// `ttymap.plugin` userdata exposed to setup-phase Lua states only.
/// Plugin scripts call `ttymap.plugin:open()` from inside a palette /
/// keybind callback to ask the host to push a fresh component onto
/// the compositor stack, and `:close()` to ask the host to close
/// whichever instance is currently on the stack. Both are
/// fire-and-forget: the request flips an atomic, and Rust drains it
/// at the next sync point (post-callback for `open`, the next
/// `Component::poll` for `close`).
struct HostPlugin {
    ctl: PluginCtl,
}

impl UserData for HostPlugin {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("open", |_, this, _: ()| {
            this.ctl
                .open_request
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        });
        methods.add_method("close", |_, this, _: ()| {
            this.ctl
                .close_request
                .store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        });
    }
}

// ── ttymap.window ─────────────────────────────────────────────────────

struct HostWindow {
    close_tx: mpsc::Sender<()>,
    export_tx: mpsc::Sender<()>,
}

impl UserData for HostWindow {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.window:close()` — fire-and-forget request to pop the
        // component off the compositor stack. The Lua side calls
        // this when its work is done (e.g. one-shot here-jump);
        // [`LuaComponent`] drains the channel after the current
        // callback returns and invokes `Window::close()` while it
        // still holds the borrow.
        methods.add_method("close", |_, this, _: ()| {
            let _ = this.close_tx.send(());
            Ok(())
        });

        // `ttymap.window:export_frame()` — fire-and-forget request to
        // emit `AppMsg::ExportFrame`, which `App::dispatch` translates
        // into "snapshot the current `MapFrame` to disk as ANSI".
        // Drained in lockstep with jump/close after each Lua callback
        // while a `Window` is still in scope.
        methods.add_method("export_frame", |_, this, _: ()| {
            let _ = this.export_tx.send(());
            Ok(())
        });
    }
}

// ── ttymap.json ───────────────────────────────────────────────────────

struct HostJson;

impl UserData for HostJson {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.json:parse(s) -> value | nil` — turn a JSON string
        // into nested Lua tables. Objects become string-keyed tables,
        // arrays become 1-indexed tables, `null` is `nil`. Parse
        // errors return `nil` and log a warning, so a flaky upstream
        // doesn't crash a plugin.
        methods.add_method(
            "parse",
            |lua, _this, source: String| match serde_json::from_str::<serde_json::Value>(&source) {
                Ok(v) => json_to_lua(lua, &v).map(Some),
                Err(e) => {
                    log::warn!("lua-host: json:parse failed: {}", e);
                    Ok(None)
                }
            },
        );
    }
}

/// Recursive translation of a `serde_json::Value` into a
/// `mlua::Value`. Objects map to string-keyed tables, arrays to
/// 1-indexed tables (Lua convention), null to nil, integers to
/// `Integer` when they fit and `Number` otherwise.
fn json_to_lua(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    match value {
        serde_json::Value::Null => Ok(mlua::Value::Nil),
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                // Numbers that fit neither i64 nor f64 are
                // exotic (large unsigned). Surface as nil rather
                // than panic; plugins can do their own handling.
                Ok(mlua::Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(items) => {
            let table = lua.create_table()?;
            // Lua arrays are 1-indexed.
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, item)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
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

        // `ttymap.help:palette_entries() -> [{name, key, label,
        // footer_hints}, …]` — snapshot of every plugin's metadata,
        // appended during registration. Read lazily so help can be
        // loaded mid-registration and still see every sibling at
        // render time. `footer_hints` is a 1-indexed list of
        // `{key, label}` rows mirroring the plugin's
        // `module.footer_hints` declaration; empty when the script
        // omits the field. Returns an empty list when the snapshot
        // hasn't been populated yet.
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
                let hints = lua.create_table()?;
                for (j, (k, v)) in entry.footer_hints.iter().enumerate() {
                    let hint = lua.create_table()?;
                    hint.set("key", k.as_str())?;
                    hint.set("label", v.as_str())?;
                    hints.set(j + 1, hint)?;
                }
                row.set("footer_hints", hints)?;
                table.set(i + 1, row)?;
            }
            Ok(table)
        });
    }
}

// ── Job ─────────────────────────────────────────────────────────────

/// One-shot fetch handle. Stays alive in the Lua state until the
/// plugin drops its reference (or until the Lua state itself is
/// dropped, which happens when the LuaComponent is rebuilt).
pub struct LuaJob {
    rx: mpsc::Receiver<String>,
}

impl LuaJob {
    /// Background HTTP GET. With `cache_ttl == None` it's a plain
    /// fetch; with `Some(ttl_secs)` it's read-through against a
    /// disk cache (write-through on success, stale-fallback on HTTP
    /// error so a rate-limiting upstream doesn't strand callers on
    /// "no body, no error"). Cache miss / stale rolls into the
    /// network path automatically.
    fn spawn(http: &HttpClient, url: String, cache_ttl: Option<u64>) -> Self {
        let (tx, rx) = mpsc::channel();
        let http = http.clone();
        let path = cache_ttl.and_then(|_| http_cache_path(&url));
        thread::spawn(move || {
            // Fresh cache hit → return immediately, skip the network.
            if let (Some(ttl), Some(p)) = (cache_ttl, path.as_ref())
                && let Ok(meta) = std::fs::metadata(p)
                && let Ok(modified) = meta.modified()
                && let Ok(age) = SystemTime::now().duration_since(modified)
                && age.as_secs() < ttl
                && let Ok(body) = std::fs::read_to_string(p)
            {
                let _ = tx.send(body);
                return;
            }

            // Cache miss / stale → real fetch.
            match http.get_bytes(&url) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(body) => {
                        if let Some(p) = path.as_ref() {
                            if let Some(parent) = p.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(p, &body);
                        }
                        let _ = tx.send(body);
                    }
                    Err(e) => log::warn!("lua-host: http:fetch {}: not utf-8: {}", url, e),
                },
                Err(e) => {
                    log::warn!("lua-host: http:fetch {}: {}", url, e);
                    // Cached caller? Try to serve a stale copy so a
                    // flaky upstream doesn't strand them.
                    if let Some(p) = path.as_ref()
                        && let Ok(body) = std::fs::read_to_string(p)
                    {
                        log::info!("lua-host: http:fetch {}: using stale cache", url);
                        let _ = tx.send(body);
                    }
                }
            }
        });
        Self { rx }
    }
}

/// Where we stash an HTTP response for `fetch_cached`. FNV-1a 64-bit
/// keeps the cache key deterministic across runs (Rust's default
/// hasher is not), and small enough to fit on every filesystem.
/// `None` means we couldn't resolve a per-user cache dir — the
/// caller treats it as a permanent miss + no write.
fn http_cache_path(url: &str) -> Option<std::path::PathBuf> {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in url.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let key = format!("{:016x}", hash);
    let dir = directories::ProjectDirs::from("", "", "ttymap")?
        .cache_dir()
        .join("lua-http");
    Some(dir.join(format!("{}.txt", key)))
}

impl UserData for LuaJob {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `job:try_take() -> string | nil` — non-blocking. Returns
        // the body once it arrives, or nil while the fetch is
        // still in flight (or has failed).
        methods.add_method_mut("try_take", |_, this, _: mlua::Variadic<mlua::Value>| {
            Ok(this.rx.try_recv().ok())
        });
    }
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper for tests: install the `ttymap` table into a fresh Lua
    /// and hand back the receivers. Mirrors the production install
    /// path. The capture slot is dropped — these tests don't drive
    /// any registration, they only exercise the host-side APIs.
    fn install_for_test() -> (mlua::Lua, LuaHostHandles) {
        let lua = mlua::Lua::new();
        let slot = new_capture_slot();
        let handles = install(&lua, "lua-test", LuaHostShared::empty(), slot, None)
            .expect("install ttymap table");
        (lua, handles)
    }

    #[test]
    fn ttymap_table_is_installed_with_namespaces() {
        let (lua, _handles) = install_for_test();
        // Each namespace lookup must return a userdata; the shape
        // confirms the install wired all namespaces in.
        for ns in [
            "http", "map", "window", "json", "sgp4", "tile", "config", "help",
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
    fn host_map_jump_pushes_to_channel() {
        let (lua, handles) = install_for_test();

        // Lua-side call: longitude first, then latitude.
        lua.load("ttymap.map:jump(139.7595, 35.6828)")
            .exec()
            .expect("exec");

        let ll = handles.jump_rx.try_recv().expect("jump must be queued");
        assert!((ll.lon - 139.7595).abs() < 1e-9);
        assert!((ll.lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn host_window_close_pushes_to_channel() {
        let (lua, handles) = install_for_test();
        lua.load("ttymap.window:close()").exec().expect("exec");
        assert!(handles.close_rx.try_recv().is_ok());
    }

    #[test]
    fn host_window_export_frame_pushes_to_channel() {
        let (lua, handles) = install_for_test();
        lua.load("ttymap.window:export_frame()")
            .exec()
            .expect("exec");
        assert!(handles.export_rx.try_recv().is_ok());
    }

    #[test]
    fn url_encode_round_trips_query_chars() {
        let (lua, _handles) = install_for_test();
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
        let (lua, _handles) = install_for_test();
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
        let (lua, _handles) = install_for_test();
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
        let (lua, _handles) = install_for_test();
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
        let (lua, _handles) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("not json !")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn parse_json_null_is_nil() {
        let (lua, _handles) = install_for_test();
        let v: mlua::Value = lua
            .load(r#"return ttymap.json:parse("null")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn sgp4_namespace_propagates_iss_through_lua() {
        // End-to-end: a Lua script calls parse_tle + propagate and
        // gets a position table back. Catches bridge wiring bugs
        // (userdata borrow, namespace install, table return shape)
        // that the standalone sgp4 module tests miss.
        let (lua, _handles) = install_for_test();
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
    fn job_try_take_returns_nil_before_send() {
        // Build a job by hand (skip the HTTP path) so we can
        // assert try_take's non-blocking behaviour.
        let (tx, rx) = mpsc::channel::<String>();
        let job = LuaJob { rx };
        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(job).expect("create_userdata");
        let result: Option<String> = lua
            .load("return select(1, ...):try_take()")
            .call(ud.clone())
            .expect("call");
        assert!(result.is_none(), "try_take should be nil before send");

        // Send a value and the next try_take returns it.
        tx.send("hi".to_string()).unwrap();
        let result: Option<String> = lua
            .load("return select(1, ...):try_take()")
            .call(ud)
            .expect("call");
        assert_eq!(result.as_deref(), Some("hi"));
    }
}
