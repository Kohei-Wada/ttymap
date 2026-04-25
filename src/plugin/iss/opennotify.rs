//! Open Notify ISS endpoint — free, no-key, returns the
//! International Space Station's current position. Internal to the
//! iss plugin.
//!
//! Endpoint: `http://api.open-notify.org/iss-now.json`
//!
//! HTTP-only (no HTTPS) but used here because the HTTPS alternative
//! (`api.wheretheiss.at`) runs an old Apache/PHP that reqwest fails
//! to handshake with on some networks. The default tile source is
//! also HTTP, so this matches the project's transport baseline.
//!
//! Response shape:
//! ```json
//! { "iss_position": { "latitude": "57.70", "longitude": "-31.74" },
//!   "timestamp": 1620000000, "message": "success" }
//! ```
//! Note that latitude/longitude come back as strings, not numbers.

use log::debug;

use crate::shared::http::HttpClient;

/// Snapshot of where the ISS is right now.
#[derive(Debug, Clone, Copy)]
pub(super) struct IssPosition {
    pub lat: f64,
    pub lon: f64,
}

pub(super) struct OpenNotifyClient {
    http: HttpClient,
}

impl OpenNotifyClient {
    pub(super) fn new() -> Self {
        Self {
            http: HttpClient::new("iss"),
        }
    }

    pub(super) fn current_position(&self) -> Option<IssPosition> {
        let url = "http://api.open-notify.org/iss-now.json";
        debug!("iss: GET {}", url);

        let json: serde_json::Value = match self.http.get_json(url) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("iss: fetch failed: {}", e);
                return None;
            }
        };

        parse_position(&json)
    }
}

fn parse_position(json: &serde_json::Value) -> Option<IssPosition> {
    // open-notify returns coordinates as strings (e.g. "57.7011").
    let pos = json.get("iss_position")?;
    let lat = pos.get("latitude")?.as_str()?.parse::<f64>().ok()?;
    let lon = pos.get("longitude")?.as_str()?.parse::<f64>().ok()?;
    Some(IssPosition { lat, lon })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_response() {
        let json = serde_json::json!({
            "message": "success",
            "iss_position": { "latitude": "35.7", "longitude": "139.7" },
            "timestamp": 1_620_000_000_u64,
        });
        let p = parse_position(&json).expect("should parse");
        assert!((p.lat - 35.7).abs() < 1e-9);
        assert!((p.lon - 139.7).abs() < 1e-9);
    }

    #[test]
    fn rejects_missing_or_malformed() {
        assert!(parse_position(&serde_json::json!({})).is_none());
        assert!(
            parse_position(&serde_json::json!({ "iss_position": { "latitude": "x" } })).is_none()
        );
        assert!(
            parse_position(&serde_json::json!({ "iss_position": { "longitude": "1.0" } }))
                .is_none()
        );
    }
}
