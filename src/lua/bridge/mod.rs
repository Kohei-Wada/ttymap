//! Lua plugin → Rust trait adapters.
//!
//! Each submodule wraps a Lua module table behind one of the host's
//! Rust traits so the compositor / palette can drive it without
//! caring that the implementation is scripted:
//!
//! - [`window_component::LuaWindowComponent`] — `Component` impl,
//!   pushed by `ttymap.api.window.open`
//! - [`palette_provider::LuaPaletteProvider`] — `PaletteProvider` impl
//! - [`handle::LuaHandle`] — shared dispatch plumbing reused by both

pub mod handle;
pub mod palette_handle;
pub mod palette_provider;
pub mod window_component;
pub mod window_handle;
