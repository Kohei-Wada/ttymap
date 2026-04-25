//! Cross-cutting infrastructure shared between **host and plugin**
//! code paths. The defining test for what lives here:
//!
//! 1. used by at least one non-plugin consumer
//!    (tile fetcher, CLI command, ...)
//! 2. *and* used by at least one plugin
//!
//! Plugin-only helpers (primitives, service clients consumed
//! exclusively by plugins) live in [`crate::plugin_api`] instead.
//!
//! - `http`  — HTTP transport. Plugins (4) + tile fetcher
//! - `geoip` — IP geolocation. `here` plugin + `snap` CLI

pub mod geoip;
pub mod http;
