//! Receptacle types for plugin → host registration.
//!
//! When a script calls `ttymap.register_palette_command(spec)` /
//! `ttymap.register_keybind(key, fn)` / `ttymap.on_event(name, fn)`
//! at setup time, the host doesn't act on those calls immediately —
//! it captures them into a [`CaptureSlot`] and consumes the captured
//! [`CapturedRegistration`] after `lua.load(source).exec()` returns.
//!
//! Not a Lua namespace — these are Rust holding types. Lives at
//! `lua/` (sibling of `lua/host.rs`) so the `api/` directory stays
//! pure 1:1 with Lua namespaces.
//!
//! The capturers themselves (the Rust closures registered on the
//! `ttymap` table) live in [`crate::lua::api::register`] and
//! [`crate::lua::api::imperative`].

use std::cell::RefCell;
use std::rc::Rc;

use mlua::RegistryKey;

/// One palette row declared by a plugin via
/// `ttymap.register_palette_command(spec)`. The `invoke` callback is
/// stored as a [`RegistryKey`] so it survives the registration call
/// and can be invoked from the persistent Lua state at activation
/// time. The state must be kept alive (held by the registrar) for
/// the program lifetime.
pub struct PaletteCommandSpec {
    pub label: String,
    pub hint: String,
    pub invoke: RegistryKey,
}

/// One keybind declared via `ttymap.register_keybind(key, callback)`.
/// `key` is a single Char activation; `callback` runs at press time
/// and (truthy return) opts into pushing the file's plugin component.
pub struct KeybindSpec {
    pub key: char,
    pub callback: RegistryKey,
}

/// One subscription declared via `ttymap.on_event(name, fn)` (or its
/// `ttymap.api.frame.on_tick(fn)` sugar, which lowers to event name
/// `"tick"`). The host walks these at register time and pushes one
/// [`Subscriber::Lua`](crate::event::Subscriber) into the
/// [`EventBus`](crate::event::EventBus) bucket for `event_name`.
pub struct EventSubscription {
    pub event_name: &'static str,
    pub callback: RegistryKey,
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
/// passes one to [`crate::lua::api::install`] and reads it back after
/// running the script.
pub fn new_capture_slot() -> CaptureSlot {
    Rc::new(RefCell::new(CapturedRegistration::default()))
}
