//! Where The ISS At — free, no-key endpoint that returns the
//! International Space Station's current position. Internal to the iss
//! plugin.
//!
//! Endpoint: `https://api.wheretheiss.at/v1/satellites/25544`
//! (NORAD id 25544 = ISS).

use log::debug;

use crate::shared::http::HttpClient;

/// Snapshot of where the ISS is right now. Velocity / altitude are
/// kept off this struct until a MapApi primitive can do something
/// with them — same minimum-viable rule we used for aircraft.
#[derive(Debug, Clone, Copy)]
pub(super) struct IssPosition {
    pub lat: f64,
    pub lon: f64,
}

pub(super) struct WhereTheIssAtClient {
    http: HttpClient,
}

impl WhereTheIssAtClient {
    pub(super) fn new() -> Self {
        Self {
            http: HttpClient::new("iss"),
        }
    }

    pub(super) fn current_position(&self) -> Option<IssPosition> {
        let url = "https://api.wheretheiss.at/v1/satellites/25544";
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
    let lat = json.get("latitude")?.as_f64()?;
    let lon = json.get("longitude")?.as_f64()?;
    Some(IssPosition { lat, lon })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_response() {
        let json = serde_json::json!({
            "name": "iss",
            "id": 25544,
            "latitude": 35.7,
            "longitude": 139.7,
            "altitude": 408.0,
            "velocity": 27600.0,
        });
        let p = parse_position(&json).expect("should parse");
        assert!((p.lat - 35.7).abs() < 1e-9);
        assert!((p.lon - 139.7).abs() < 1e-9);
    }

    #[test]
    fn rejects_missing_fields() {
        assert!(parse_position(&serde_json::json!({})).is_none());
        assert!(parse_position(&serde_json::json!({ "latitude": 1.0 })).is_none());
        assert!(parse_position(&serde_json::json!({ "longitude": 1.0 })).is_none());
    }
}
