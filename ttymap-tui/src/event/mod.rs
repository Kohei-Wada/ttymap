//! Event subsystem — Lua-agnostic pub/sub primitive.
//!
//! Replaces the previous `lua/registry.rs`'s `LuaEventBus`. The bus
//! dispatches on the **main thread** (`mlua::Lua` is `!Send`, so Lua
//! callbacks must run there), but **publishers can run anywhere**:
//! cross-thread producers wrap an [`Event`] in
//! [`crate::app::AppEvent::Bus`] and push onto the App-level mpsc;
//! the main loop drains and calls [`EventBus::publish`].
//!
//! Subscribers come in two flavours:
//! - [`Subscriber::Rust`] — a closure invoked with `&Event`.
//! - [`Subscriber::Lua`] — an `mlua::Function` invoked with the
//!   event's typed Lua args (`map_jumped` → `(lon, lat)`, etc).
//!
//! The `tick` per-frame draw hook is **not** an [`Event`] variant —
//! it requires a borrowed `MapApi` for that frame and lives in
//! [`EventBus::dispatch_tick`].
//!
//! Pattern modelled after Yazi's `yazi-dds` (Rust + Lua TUI with
//! exactly this constraint set).

pub mod bus;
pub mod payload;

pub use bus::{EventBus, Subscriber};
pub use payload::{Event, Level};
