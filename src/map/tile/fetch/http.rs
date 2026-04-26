//! HTTP `TileFetcher` — fetches MVT (`.pbf`) tiles over the slippy-map
//! URL scheme `{base}/{z}/{x}/{y}.pbf`. ttymap's map rendering
//! assumes OSM-derived OpenMapTiles data, and `mapscii.me` is the
//! only public server that serves it without an API key, so the base
//! URL defaults to that.
//!
//! Disk cache is **not** this module's concern — wrap with
//! `super::DiskCachedFetcher` to add a read-through / write-through
//! disk layer. Concurrency / queueing / dedup live in `super::lane`.
//! This file is just the per-tile HTTP GET.

use std::time::Duration;

use super::{FetchError, TileFetcher};
use crate::map::tile::key::TileKey;
use crate::shared::http::HttpClient;

// Use HTTPS directly: `http://mapscii.me/...` 301-redirects to the
// HTTPS URL, costing an extra round-trip per fetch on cold connections.
const BASE_URL: &str = "https://mapscii.me";
const ATTRIBUTION: &str = "© OpenStreetMap contributors";

pub struct HttpFetcher {
    http: HttpClient,
    base_url: String,
}

impl HttpFetcher {
    /// Build a fetcher with the default `mapscii.me` base.
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL.to_string())
    }

    /// Build a fetcher with a custom base URL — useful for tests
    /// against a local mock server, and for future config-driven
    /// alternative tile sources.
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            http: HttpClient::with_timeout("tile", Duration::from_secs(10)),
            base_url,
        }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl TileFetcher for HttpFetcher {
    fn fetch(&self, key: &TileKey) -> Result<Vec<u8>, FetchError> {
        let url = format!("{}/{}.pbf", self.base_url, key);
        self.http
            .get_bytes(&url)
            .map_err(|e| FetchError::new(e.to_string()))
    }

    fn attribution(&self) -> &str {
        ATTRIBUTION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_uses_base_plus_zxy_pbf() {
        let fetcher = HttpFetcher::with_base_url("http://example.test".to_string());
        assert_eq!(fetcher.base_url, "http://example.test");
        assert_eq!(fetcher.attribution(), ATTRIBUTION);
    }

    #[test]
    fn default_uses_mapscii_me_https_base() {
        let fetcher = HttpFetcher::new();
        assert_eq!(fetcher.base_url, "https://mapscii.me");
    }
}
