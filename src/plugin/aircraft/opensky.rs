//! OpenSky Network HTTP client — anonymous access to live aircraft
//! state vectors (ADS-B). Internal to the aircraft plugin.
//!
//! Endpoint: `https://opensky-network.org/api/states/all`
//!
//! Anonymous usage costs 4 credits per unbounded call and 1 credit per
//! bounding-box-restricted call. The plugin always passes a bbox
//! derived from the map centre to stay cheap.

use log::debug;

use crate::shared::http::HttpClient;

/// One state vector — what OpenSky returns per aircraft. Optional
/// fields are kept Option-typed because the API freely emits null
/// for fields a fresh track hasn't reported yet (no callsign, no
/// altitude lock, etc.).
#[derive(Debug, Clone)]
pub(super) struct Aircraft {
    pub callsign: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<f64>,
    pub velocity_ms: Option<f64>,
    /// True track in degrees (0 = north, clockwise). `None` when
    /// the aircraft hasn't reported heading yet.
    pub heading_deg: Option<f64>,
    pub on_ground: bool,
}

pub(super) struct OpenSkyClient {
    http: HttpClient,
}

impl OpenSkyClient {
    pub(super) fn new() -> Self {
        Self {
            http: HttpClient::new("opensky"),
        }
    }

    /// Fetch all aircraft within a bounding box centred on (lat, lon)
    /// with `half_deg` margin on each side.
    pub(super) fn states_around(&self, lat: f64, lon: f64, half_deg: f64) -> Vec<Aircraft> {
        let lamin = (lat - half_deg).max(-90.0);
        let lamax = (lat + half_deg).min(90.0);
        let lomin = (lon - half_deg).clamp(-180.0, 180.0);
        let lomax = (lon + half_deg).clamp(-180.0, 180.0);
        let url = format!(
            "https://opensky-network.org/api/states/all?\
             lamin={lamin}&lomin={lomin}&lamax={lamax}&lomax={lomax}"
        );
        debug!("opensky: GET {}", url);

        let json: serde_json::Value = match self.http.get_json(&url) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("opensky: fetch failed: {}", e);
                return Vec::new();
            }
        };

        parse_states(&json)
    }
}

/// Parse the OpenSky `states/all` response. The state vector is a
/// fixed-position array; indices are documented at
/// https://openskynetwork.github.io/opensky-api/rest.html.
fn parse_states(json: &serde_json::Value) -> Vec<Aircraft> {
    let Some(states) = json.get("states").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    states.iter().filter_map(parse_one).collect()
}

fn parse_one(state: &serde_json::Value) -> Option<Aircraft> {
    let arr = state.as_array()?;
    // Required: longitude (5), latitude (6). Anything else is null-
    // tolerated; on_ground (8) defaults to false when absent.
    let callsign = arr
        .get(1)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let lon = arr.get(5)?.as_f64()?;
    let lat = arr.get(6)?.as_f64()?;
    let altitude_m = arr.get(7).and_then(|v| v.as_f64());
    let on_ground = arr.get(8).and_then(|v| v.as_bool()).unwrap_or(false);
    let velocity_ms = arr.get(9).and_then(|v| v.as_f64());
    let heading_deg = arr.get(10).and_then(|v| v.as_f64());
    Some(Aircraft {
        callsign,
        lat,
        lon,
        altitude_m,
        velocity_ms,
        heading_deg,
        on_ground,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "time": 1_700_000_000_u64,
            "states": [
                ["abc123", "JAL123  ", "Japan", 1, 1, 139.7, 35.7, 10000.0, false, 250.0, 90.0, 0.0, null, 10100.0, "1234", false, 0],
                ["def456", null,       "USA",   1, 1, -73.9, 40.7,  null,    true,  null,  null, null, null,    null,    null,    false, 0],
                ["bad",   null,       "?",     1, 1, null,  null,  null,    false, null,  null, null, null,    null,    null,    false, 0],
            ]
        })
    }

    #[test]
    fn parses_well_formed_states() {
        let parsed = parse_states(&sample_response());
        assert_eq!(parsed.len(), 2, "third row missing lat/lon should drop");

        let a = &parsed[0];
        assert!((a.lat - 35.7).abs() < 1e-9);
        assert!((a.lon - 139.7).abs() < 1e-9);
        assert!(!a.on_ground);

        let b = &parsed[1];
        assert!(b.on_ground);
    }

    #[test]
    fn empty_or_missing_states_yields_empty() {
        assert!(parse_states(&serde_json::json!({})).is_empty());
        assert!(parse_states(&serde_json::json!({ "states": null })).is_empty());
        assert!(parse_states(&serde_json::json!({ "states": [] })).is_empty());
    }
}
