//! Cross-cutting infrastructure shared across domains.
//!
//! Anything that is not tied to a particular subsystem (tile, render, ui, …)
//! but is reused by several of them lives here. Today this hosts the HTTP
//! transport plus a couple of service clients (geoip, nominatim) that
//! straddle plugin and host code. Plugin-author primitives (`AsyncJob`,
//! `Throttle`, `PolledFeed`) live in [`crate::plugin_api`] instead.

pub mod geoip;
pub mod http;
pub mod nominatim;
