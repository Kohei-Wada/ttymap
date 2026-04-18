//! Cross-cutting infrastructure shared across domains.
//!
//! Anything that is not tied to a particular subsystem (tile, render, ui, …)
//! but is reused by several of them lives here. Today this is just the HTTP
//! client factory; future candidates include retry policies, rate-limiters,
//! or shared middleware.

pub mod http;
pub mod nominatim;
pub mod throttle;
