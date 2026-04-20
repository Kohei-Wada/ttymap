//! IP-based geolocation — single-shot lookup used at startup when the
//! user passes `--here` (see issue #44). Returns approximate `(lat, lon)`
//! for the outbound IP based on a public lookup service.
//!
//! Default endpoint is `https://ipapi.co/json/`, which returns
//! `{"latitude": <f64>, "longitude": <f64>, ...}`. The endpoint is
//! overridable via config for users who want to point at a self-hosted
//! or alternative service that follows the same key shape.

use std::time::Duration;

use log::debug;

use crate::shared::http::HttpClient;

/// Look up approximate `(lat, lon)` for the outbound IP. Returns `None`
/// on any network / parse error or rate-limit response; callers fall
/// back to their configured default.
pub fn lookup(endpoint: &str, timeout_ms: u64) -> Option<(f64, f64)> {
    let http = HttpClient::with_timeout("geoip", Duration::from_millis(timeout_ms));
    debug!("geoip: lookup {}", endpoint);

    let json: serde_json::Value = http.get_json(endpoint).ok()?;

    // Some endpoints (e.g. ipapi.co when rate-limited) return HTTP 200
    // with `{"error": true, "reason": "..."}`.
    if json.get("error").and_then(|v| v.as_bool()).unwrap_or(false) {
        debug!("geoip: endpoint returned error flag: {}", json);
        return None;
    }

    let lat = json.get("latitude")?.as_f64()?;
    let lon = json.get("longitude")?.as_f64()?;
    Some((lat, lon))
}
