//! Host-side runtime state surfaced to every Lua plugin.
//!
//! Not a Lua namespace — these are Rust structs that the
//! [`crate::lua::api`] namespace userdatas read from / write to.
//! Lives at `lua/` (not `lua/api/`) so the `api/` directory stays
//! pure 1:1 with Lua namespaces.
//!
//! - [`LuaHostShared`] — read-mostly snapshot (attribution, geoip
//!   endpoint, keymap rows, palette entries, notification ring).
//!   Built once in [`crate::app::App::new`] and Arc-cloned into
//!   each namespace userdata.
//! - [`LuaHostHandles`] — per-plugin handle pair returned by
//!   [`crate::lua::api::install`]; the host refreshes `center` /
//!   `zoom` from any dispatch path that carries a `Window`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ttymap_engine::geo::LonLat;

/// Maximum number of pending notifications retained in the host's
/// shared ring buffer. Sized to absorb a brief flurry (a search
/// returning, a fetch erroring, a file exporting) without needing
/// per-call resizing — at typical 3-second display TTL the buffer
/// rarely hits cap, and on overflow we drop the oldest so newer
/// signals are never starved.
pub(crate) const NOTIFY_RING_CAP: usize = 16;

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
    /// IP-geolocation endpoint URL (`ttymap.opt.geoip.endpoint` in
    /// `init.lua`). The here plugin GETs this to resolve the
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

/// Channels + shared state owned by **the setup state** (the Lua VM
/// that runs the script's top-level `register_*` calls and continues
/// to run palette / keybind callbacks for the program lifetime).
/// [`crate::lua::api::install`] returns this once per state; the App
/// routes the shared cells to the right consumers.
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
