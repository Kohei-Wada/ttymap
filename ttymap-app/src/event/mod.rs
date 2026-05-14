//! Event subsystem — Lua-agnostic pub/sub primitive.
//!
//! Replaces the previous `lua/registry.rs`'s `LuaEventBus`. The bus
//! dispatches on the **main thread** (`mlua::Lua` is `!Send`, so Lua
//! callbacks must run there), but **publishers can run anywhere**:
//! cross-thread producers wrap an [`Event`] in
//! [`crate::app::AppEvent::Bus`] and push onto the App-level mpsc;
//! the main loop drains and calls [`EventBus::publish`].
//!
//! Subscribers are plain `Fn(&Event)` closures stored as
//! `Rc<dyn Fn>`. Lua plugins subscribe through
//! `ttymap.on_event(name, fn)` (see `crate::lua::api::register`),
//! which wraps the Lua callback in a Rust closure that captures the
//! `mlua::Lua` + `RegistryKey` and handles the Lua call site —
//! keeping `event/` free of any `mlua` import.
//!
//! The `tick` per-frame draw hook is **not** an [`Event`] variant
//! and is **not** routed through the bus at all — it requires a
//! borrowed `MapApi` that can't fit `&Event`, and lives in
//! `crate::lua::tick` with its own `TickRegistry`.
//!
//! Pattern modelled after Yazi's `yazi-dds` (Rust + Lua TUI with
//! exactly this constraint set).

pub mod bus;
pub mod payload;

pub use bus::EventBus;
pub use payload::{Event, Level};
