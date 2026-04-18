//! Nominatim API client — forward and reverse geocoding.
//! https://nominatim.openstreetmap.org/

use log::debug;

use crate::geo::LonLat;

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

/// Nominatim HTTP client.
pub struct NominatimClient {
    client: reqwest::blocking::Client,
}

impl NominatimClient {
    pub fn new() -> Option<Self> {
        let client = crate::shared::http::client_builder().build().ok()?;
        Some(Self { client })
    }

    /// Forward geocoding: place name → coordinates.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let url = format!(
            "{}/search?q={}&format=json&limit=5",
            BASE_URL,
            urlencoded(query),
        );
        debug!("nominatim: search {}", url);

        let json: Vec<serde_json::Value> = match self.get_json(&url) {
            Some(v) => v,
            None => return Vec::new(),
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

        let json: serde_json::Value = self.get_json(&url)?;
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

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Option<T> {
        let response = match self.client.get(url).send() {
            Ok(r) => r,
            Err(e) => {
                debug!("nominatim: request error: {}", e);
                return None;
            }
        };
        if !response.status().is_success() {
            debug!("nominatim: status {}", response.status());
            return None;
        }
        match response.json() {
            Ok(j) => Some(j),
            Err(e) => {
                debug!("nominatim: parse error: {}", e);
                None
            }
        }
    }
}

/// Simple percent-encoding for query strings.
fn urlencoded(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
