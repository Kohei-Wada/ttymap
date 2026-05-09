//! Receptacle types for plugin → host registration.
//!
//! When a script calls `ttymap.register_palette_command(spec)` /
//! `ttymap.register_keybind(key, fn)` at setup time, the host doesn't
//! act on those calls immediately — it captures them into a
//! [`CaptureSlot`] and consumes the captured [`CapturedRegistration`]
//! after `lua.load(source).exec()` returns.
//!
//! `ttymap.on_event(name, fn)` and `ttymap.api.frame.on_tick(fn)` are
//! **not** captured — they subscribe directly against the
//! [`EventBus`](crate::event::EventBus) at call time and return a
//! Lua-facing `EventHandle` so plugins can `:remove()` later. The
//! [`Self::events_registered`] counter just tracks how many events
//! the script subscribed during load so [`load_chunk`] can keep its
//! "must subscribe to something" gate intact for tick-only plugins.
//!
//! Not a Lua namespace — these are Rust holding types. Lives at
//! `lua/` (sibling of `lua/host.rs`) so the `api/` directory stays
//! pure 1:1 with Lua namespaces.
//!
//! The capturers themselves (the Rust closures registered on the
//! `ttymap` table) live in [`crate::lua::api::register`] and
//! [`crate::lua::api::imperative`].
//!
//! [`load_chunk`]: crate::lua::bridge::handle::load_chunk

use std::cell::RefCell;
use std::rc::Rc;

use mlua::RegistryKey;

/// One palette row declared by a plugin via
/// `ttymap.register_palette_command(spec)`. The `invoke` callback is
/// stored as a [`RegistryKey`] so it survives the registration call
/// and can be invoked from the persistent Lua state at activation
/// time. The state must be kept alive (held by the registrar) for
/// the program lifetime.
///
/// `id` is the same monotonic ID the corresponding
/// [`crate::lua::bridge::registrar_handle::PaletteCommandHandle`]
/// returned to Lua holds — so when the plugin calls `:remove()` the
/// host can find this entry in the live registry and drop it.
pub struct PaletteCommandSpec {
    pub id: u64,
    pub label: String,
    pub hint: String,
    pub invoke: RegistryKey,
}

/// One keybind declared via `ttymap.register_keybind(key, callback)`.
/// `key` is a single Char activation; `callback` runs at press time
/// and (truthy return) opts into pushing the file's plugin component.
///
/// `id` is the matching handle ID; see [`PaletteCommandSpec::id`].
pub struct KeybindSpec {
    pub id: u64,
    pub key: char,
    pub callback: RegistryKey,
}

/// Everything a single plugin file's setup phase declared. nvim-
/// style: each activation surface is a separate explicit call with
/// its own Lua callback. Plugins own whether/when to push by
/// inspecting their own state inside the callback and calling
/// `ttymap.api.card.open(spec)` / `ttymap.api.palette.open(spec)`.
/// Per-frame work subscribes via `ttymap.api.frame.on_tick(fn)` —
/// stacked: each call appends a callback that fires every frame.
/// Other events go through `ttymap.on_event(name, fn)`. Both
/// subscribe directly against the [`EventBus`] and bump
/// [`Self::events_registered`] so the load gate sees them.
#[derive(Default)]
pub struct CapturedRegistration {
    /// Each `ttymap.register_palette_command({label, invoke})` call.
    pub palette_commands: Vec<PaletteCommandSpec>,
    /// Each `ttymap.register_keybind(key, callback)` call.
    pub keybinds: Vec<KeybindSpec>,
    /// Plugin name being loaded right now. Set by [`load_chunk`]
    /// before executing the script and used by `on_tick` /
    /// `on_event` capturers to attribute the bus subscriber to the
    /// right plugin (for log prefixes / debugging). `None` outside a
    /// `load_chunk` call.
    ///
    /// [`load_chunk`]: crate::lua::bridge::handle::load_chunk
    pub current_plugin: Option<&'static str>,
    /// Count of `ttymap.on_event` / `ttymap.api.frame.on_tick` calls
    /// the script made during this load. Used purely as a "did the
    /// script subscribe to anything?" signal by [`load_chunk`]'s
    /// gate; the actual subscriptions live on the
    /// [`EventBus`](crate::event::EventBus).
    pub events_registered: usize,
}

/// Slot used by a fresh Lua state to capture the script's
/// registration calls. `Rc<RefCell<...>>` is fine — the Lua state
/// is single-threaded and the capture lifetime is bounded by
/// `lua.load(source).exec()`.
pub type CaptureSlot = Rc<RefCell<CapturedRegistration>>;

/// Build an empty capture slot. The caller passes one to
/// [`crate::lua::api::install`] (once for the whole subsystem) and
/// drains it via [`crate::lua::bridge::handle::load_chunk`] after
/// each plugin's `exec` so registrations are attributed per-plugin.
pub fn new_capture_slot() -> CaptureSlot {
    Rc::new(RefCell::new(CapturedRegistration::default()))
}
