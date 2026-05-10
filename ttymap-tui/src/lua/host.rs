//! Host-side runtime state surfaced to every Lua plugin.
//!
//! Not a Lua namespace — these are Rust structs that the
//! [`crate::lua::api`] namespace userdatas read from / write to.
//! Lives at `lua/` (not `lua/api/`) so the `api/` directory stays
//! pure 1:1 with Lua namespaces.
//!
//! - [`LuaHostShared`] — read-mostly snapshot (attribution, geoip
//!   endpoint, keymap rows, palette entries). Built once in
//!   [`crate::app::App::new`] and Arc-cloned into each namespace
//!   userdata.
//! - [`LuaHostHandles`] — per-plugin handle pair returned by
//!   [`crate::lua::api::install`]; the host refreshes `center` /
//!   `zoom` from any dispatch path that carries a `Window`.

use std::sync::{Arc, Mutex};

use ttymap_engine::geo::LonLat;
use ttymap_engine::map::render::frame::MapFrame;

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
    /// without OSM data, mostly). Held behind a `Mutex` because the
    /// host builds the tile cache *after* `build_subsystem` (the
    /// engine needs the parsed `Config`, which the Lua bootstrap
    /// produces); the binary calls [`Self::set_attribution`] once
    /// the cache is up.
    pub attribution: Mutex<Option<String>>,
    /// IP-geolocation endpoint URL (`ttymap.opt.geoip.endpoint` in
    /// `init.lua`). The here plugin GETs this to resolve the
    /// user's coordinates.
    pub geoip_endpoint: String,
    /// `(key-binding, action-label)` pairs for built-in map actions.
    /// Help renders this as the keymap section of its cheatsheet.
    /// Held behind a `Mutex` because plugins may load (via the init.lua
    /// `require` chain) before the host has parsed the user's
    /// `ttymap.keymap.set/del` mutations and built the live `KeyMap` —
    /// the entries are populated post-init.lua via [`Self::set_keymap_entries`].
    /// Help reads lazily at render time, so the brief register-time
    /// emptiness is invisible.
    pub keymap_entries: Mutex<Vec<(String, String)>>,
    /// Help-cheatsheet rows, appended during plugin registration
    /// (one per `register_palette_command` call that supplied a
    /// non-empty `hint`). Held behind a `Mutex` so `LuaHostShared`
    /// can be Arc'd into each plugin's host namespaces at register
    /// time and populated later. Help reads this lazily (at render
    /// time, not register time) so it sees every entry regardless of
    /// load order.
    pub help_entries: Mutex<Vec<HelpEntry>>,
    /// Latest [`MapFrame`] drained from the render thread, mirrored
    /// here for Lua's read side (`ttymap.api.frame.to_ansi()`).
    /// `None` until the first frame arrives. App refreshes this on
    /// every `AppEvent::FrameReady` via [`crate::lua::LuaHandle::set_current_frame`];
    /// it never crosses threads (single main-thread accessor) but
    /// uses `Mutex` for shape-uniformity with the other shared
    /// fields and to keep `LuaHostShared` Sync.
    pub current_frame: Mutex<Option<MapFrame>>,
}

/// One help-cheatsheet row. Surfaced to Lua via
/// `ttymap.help:palette_entries()` so help.lua can render it without
/// caring about how the data was harvested. Only `register_palette_command`
/// calls with a non-empty `hint` (the keybind char) land here; keyless
/// palette-only entries are filtered at push time.
#[derive(Clone)]
pub struct HelpEntry {
    pub key: String,
    pub label: String,
}

impl LuaHostShared {
    pub fn new(geoip_endpoint: String) -> Self {
        Self {
            attribution: Mutex::new(None),
            geoip_endpoint,
            keymap_entries: Mutex::new(Vec::new()),
            help_entries: Mutex::new(Vec::new()),
            current_frame: Mutex::new(None),
        }
    }

    /// Set the tile provider's attribution string. Called once by
    /// the binary after the tile cache spins up; reads from
    /// `ttymap.tile:attribution()` see the new value next call.
    pub fn set_attribution(&self, attribution: Option<String>) {
        if let Ok(mut slot) = self.attribution.lock() {
            *slot = attribution;
        }
    }

    /// Append one help-cheatsheet row. Called from
    /// `register_palette_command` when the spec has a non-empty
    /// `hint`. A poisoned mutex is silently skipped — losing a help
    /// row is preferable to crashing the host.
    pub fn push_help_entry(&self, entry: HelpEntry) {
        if let Ok(mut slot) = self.help_entries.lock() {
            slot.push(entry);
        }
    }

    /// Replace the keymap-entries snapshot. Called once during
    /// bootstrap, after init.lua has run (its `ttymap.keymap.set/del`
    /// mutations land in `KeybindingOverrides`, which the binary
    /// folds into a live `KeyMap` and serialises to entries here).
    pub fn set_keymap_entries(&self, entries: Vec<(String, String)>) {
        if let Ok(mut slot) = self.keymap_entries.lock() {
            *slot = entries;
        }
    }

    /// All-empty default for tests and registration-time loads that
    /// don't need real runtime data. The `ttymap.*` host surface
    /// still installs in a Lua state used only to capture the
    /// script's `register_*` call.
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::new(String::new()))
    }
}

/// Channels + shared state owned by the **shared Lua VM** (the
/// single Lua state that runs `init.lua`, every plugin's top-level
/// `register_*` calls, and every plugin callback for the program
/// lifetime). [`crate::lua::api::install`] returns this **once** for
/// the whole subsystem; every plugin reads the same `center` /
/// `zoom` cells.
///
/// - **UserCommand sender** (not part of these handles) — every
///   fire-and-forget Lua intent (`ttymap.map:jump` / `:zoom(level)` /
///   `:fly_to` / `ttymap.api.frame.export`) is pre-built into an
///   [`crate::UserCommand`] on the Lua side and pushed through a
///   `Sender<UserCommand>` that every plugin clones from the **single**
///   App-level channel. The receiver lives directly on `App`; a single
///   drain per frame covers every plugin's intents.
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
    /// `ttymap.map` userdata (`HostMap`); refreshed on the dispatch
    /// paths that also refresh `center`.
    pub zoom: Arc<Mutex<f64>>,
}
