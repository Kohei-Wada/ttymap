//! USGS earthquake feed — GeoJSON of recent quakes. Internal to the
//! quake plugin.
//!
//! Endpoint: `https://earthquake.usgs.gov/earthquakes/feed/v1.0/\
//! summary/2.5_day.geojson` — magnitude 2.5+ in the past 24h
//! (≈40–60 events on a normal day, all-world).
//!
//! Per-feature shape:
//! ```json
//! { "properties": { "mag": 4.2, ... },
//!   "geometry":   { "coordinates": [lon, lat, depth_km] } }
//! ```

use log::debug;

use crate::shared::http::HttpClient;

const FEED_URL: &str = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson";

/// One earthquake in the feed.
///
/// `depth_km` and place/time fields exist in the response but are
/// kept off this struct until MapApi gains primitives (graded color,
/// label) that can render them — same minimum-viable rule as
/// aircraft.
#[derive(Debug, Clone, Copy)]
pub(super) struct Quake {
    pub lat: f64,
    pub lon: f64,
    pub magnitude: f64,
}

pub(super) struct UsgsClient {
    http: HttpClient,
}

impl UsgsClient {
    pub(super) fn new() -> Self {
        Self {
            http: HttpClient::new("quake"),
        }
    }

    pub(super) fn recent(&self) -> Vec<Quake> {
        debug!("quake: GET {}", FEED_URL);
        let json: serde_json::Value = match self.http.get_json(FEED_URL) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("quake: fetch failed: {}", e);
                return Vec::new();
            }
        };
        parse_features(&json)
    }
}

fn parse_features(json: &serde_json::Value) -> Vec<Quake> {
    let Some(features) = json.get("features").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    features.iter().filter_map(parse_one).collect()
}

fn parse_one(feature: &serde_json::Value) -> Option<Quake> {
    let coords = feature.get("geometry")?.get("coordinates")?.as_array()?;
    let lon = coords.first()?.as_f64()?;
    let lat = coords.get(1)?.as_f64()?;
    let magnitude = feature.get("properties")?.get("mag")?.as_f64()?;
    Some(Quake {
        lat,
        lon,
        magnitude,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "properties": { "mag": 4.2 },
                    "geometry":   { "coordinates": [-119.05, 39.31, 13.16] }
                },
                {
                    "properties": { "mag": 2.6 },
                    "geometry":   { "coordinates": [139.7, 35.7, 50.0] }
                },
                {
                    "properties": { "mag": null },
                    "geometry":   { "coordinates": [0.0, 0.0, 0.0] }
                }
            ]
        })
    }

    #[test]
    fn parses_well_formed_features() {
        let parsed = parse_features(&sample());
        assert_eq!(parsed.len(), 2, "feature with null mag should drop");

        let a = &parsed[0];
        assert!((a.magnitude - 4.2).abs() < 1e-9);
        assert!((a.lat - 39.31).abs() < 1e-9);
        assert!((a.lon - (-119.05)).abs() < 1e-9);
    }

    #[test]
    fn empty_or_missing_features_yields_empty() {
        assert!(parse_features(&serde_json::json!({})).is_empty());
        assert!(parse_features(&serde_json::json!({ "features": null })).is_empty());
        assert!(parse_features(&serde_json::json!({ "features": [] })).is_empty());
    }
}
