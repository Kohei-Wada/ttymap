//! Nominatim API client — forward and reverse geocoding.
//! https://nominatim.openstreetmap.org/

use log::debug;

use crate::geo::LonLat;
use crate::shared::http::HttpClient;
use crate::shared::http::url::urlencoded;

const BASE_URL: &str = "https://nominatim.openstreetmap.org";

/// Forward geocoding result.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub location: LonLat,
}

/// Reverse geocoding result.
#[derive(Debug, Clone)]
pub struct PlaceInfo {
    pub display_name: String,
    pub city: Option<String>,
    pub country: Option<String>,
}

pub struct NominatimClient {
    http: HttpClient,
}

impl Default for NominatimClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NominatimClient {
    pub fn new() -> Self {
        Self {
            http: HttpClient::new("nominatim"),
        }
    }

    /// Forward geocoding: place name → coordinates.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let url = format!(
            "{}/search?q={}&format=json&limit=5",
            BASE_URL,
            urlencoded(query),
        );
        debug!("nominatim: search {}", url);

        let json = match self.http.get_json::<Vec<serde_json::Value>>(&url) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("nominatim: search fetch failed for \"{}\": {}", query, e);
                return Vec::new();
            }
        };

        json.iter()
            .filter_map(|item| {
                let lat: f64 = item.get("lat")?.as_str()?.parse().ok()?;
                let lon: f64 = item.get("lon")?.as_str()?.parse().ok()?;
                let name = item.get("display_name")?.as_str()?.to_string();
                Some(SearchResult {
                    name,
                    location: LonLat { lon, lat },
                })
            })
            .collect()
    }

    /// Reverse geocoding: coordinates → place info.
    pub fn reverse(&self, lat: f64, lon: f64) -> Option<PlaceInfo> {
        let url = format!(
            "{}/reverse?lat={}&lon={}&format=json&zoom=10",
            BASE_URL, lat, lon,
        );
        debug!("nominatim: reverse {}", url);

        let json: serde_json::Value = self.http.get_json(&url).ok()?;
        let display_name = json.get("display_name")?.as_str()?.to_string();
        let address = json.get("address");
        let city = address
            .and_then(|a| {
                a.get("city")
                    .or_else(|| a.get("town"))
                    .or_else(|| a.get("village"))
            })
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let country = address
            .and_then(|a| a.get("country"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(PlaceInfo {
            display_name,
            city,
            country,
        })
    }
}
