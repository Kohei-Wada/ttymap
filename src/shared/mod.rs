//! Binary-side cross-cutting utilities.
//!
//! Currently just `geoip` (IP → lat/lon resolution for the `--here`
//! flag and the `here` plugin). The engine doesn't know about
//! geoip — the binary resolves IP to a lat/lon up front, then hands
//! a plain coordinate to the engine.
//!
//! The HTTP client (User-Agent-tagged `reqwest` wrapper) lives in
//! [`ttymap_engine::shared::http`]: the engine's tile fetcher uses
//! it, and the binary's Lua `ttymap.http` bridge re-borrows it from
//! there to keep a single source of truth.

pub mod geoip;
