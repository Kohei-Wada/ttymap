//! Cross-cutting infrastructure used by both host code paths (tile
//! fetcher, CLI commands) and the Lua bridge.
//!
//! - `http`  — HTTP transport. Lua `host:fetch_url` + tile fetcher
//! - `geoip` — IP geolocation. `here` plugin + `snap` CLI

pub mod geoip;
pub mod http;
