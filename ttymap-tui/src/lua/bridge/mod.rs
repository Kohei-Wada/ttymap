//! Lua plugin → Rust trait adapters.
//!
//! Each submodule wraps a Lua module table behind one of the host's
//! Rust traits so the compositor / palette can drive it without
//! caring that the implementation is scripted:
//!
//! - [`card_component::LuaCardComponent`] — `Component` impl,
//!   pushed by `ttymap.api.card.open`
//! - [`palette_provider::LuaPaletteProvider`] — `PaletteProvider` impl
//! - [`handle::LuaBridgeHandle`] — shared dispatch plumbing reused by both
//! - [`event_handle::EventHandle`] — disposable for one bus subscription
//!   (`ttymap.on_event` / `ttymap.api.frame.on_tick`)
//! - [`registrar_handle::PaletteCommandHandle`] /
//!   [`registrar_handle::KeybindHandle`] — disposable shape for the
//!   registrar-backed surfaces (today `:remove()` is a stubbed warn,
//!   pending the live-registry refactor)

pub mod card_component;
pub mod card_handle;
pub mod card_parse;
pub mod event_handle;
pub mod handle;
pub mod palette_handle;
pub mod palette_provider;
pub mod registrar_handle;
