//! HTTP `TileFetcher` — fetches MVT (`.pbf`) tiles over the slippy-map
//! URL scheme `{base}/{z}/{x}/{y}.pbf`. ttymap's map rendering
//! assumes OSM-derived OpenMapTiles data, and `mapscii.me` is the
//! only public server that serves it without an API key, so the base
//! URL defaults to that.
//!
//! Disk cache lives here too — it's an HTTP-specific concern (other
//! backends like MBTiles are already on disk in their own format).
//! On a hit we skip the network entirely; on a miss we fetch and
//! write through.
//!
//! All concurrency / queueing / dedup lives in `super::lane`; this
//! file is just per-tile fetch + persist.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{FetchError, TileFetcher};
use crate::map::tile::key::TileKey;
use crate::shared::http::HttpClient;

const BASE_URL: &str = "http://mapscii.me";
const ATTRIBUTION: &str = "© OpenStreetMap contributors";

pub struct HttpFetcher {
    http: HttpClient,
    base_url: String,
    /// Where to look for / write persistent tile cache. `None`
    /// disables disk cache (e.g. in tests, or when the user opts out
    /// in config).
    cache_dir: Option<PathBuf>,
}

impl HttpFetcher {
    /// Build a fetcher with the default `mapscii.me` base and no
    /// disk cache.
    pub fn new() -> Self {
        Self::with_options(BASE_URL.to_string(), None)
    }

    /// Build a fetcher with the default base URL and the given disk
    /// cache directory (created lazily). Pass `None` to disable disk
    /// cache.
    pub fn with_cache_dir(cache_dir: Option<PathBuf>) -> Self {
        Self::with_options(BASE_URL.to_string(), cache_dir)
    }

    /// Build a fetcher with a custom base URL and optional disk
    /// cache — useful for tests against a local mock server, and for
    /// future config-driven alternative tile sources.
    pub fn with_options(base_url: String, cache_dir: Option<PathBuf>) -> Self {
        Self {
            http: HttpClient::with_timeout("tile", Duration::from_secs(10)),
            base_url,
            cache_dir,
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
        // 1. Disk hit short-circuits the network entirely.
        if let Some(dir) = &self.cache_dir
            && let Some(bytes) = read_disk_cache(dir, key)
        {
            return Ok(bytes);
        }

        // 2. HTTP fetch.
        let url = format!("{}/{}.pbf", self.base_url, key);
        let bytes = self
            .http
            .get_bytes(&url)
            .map_err(|e| FetchError::new(e.to_string()))?;

        // 3. Persist for next time. Best-effort — disk failures are
        //    logged via `fs::write`'s ignored result and don't block
        //    the fetch.
        if let Some(dir) = &self.cache_dir {
            write_disk_cache(dir, key, &bytes);
        }
        Ok(bytes)
    }

    fn attribution(&self) -> &str {
        ATTRIBUTION
    }
}

// ── Disk cache helpers ────────────────────────────────────────────────────────
//
// Layout: `{cache_dir}/{z}/{x}-{y}.pbf`. Same scheme as the original
// `TileCache::write_disk_cache` so existing on-disk caches keep
// working across this refactor.

fn read_disk_cache(dir: &Path, key: &TileKey) -> Option<Vec<u8>> {
    fs::read(
        dir.join(key.z.to_string())
            .join(format!("{}-{}.pbf", key.x, key.y)),
    )
    .ok()
}

fn write_disk_cache(dir: &Path, key: &TileKey, bytes: &[u8]) {
    let tile_dir = dir.join(key.z.to_string());
    let _ = fs::create_dir_all(&tile_dir);
    let _ = fs::write(tile_dir.join(format!("{}-{}.pbf", key.x, key.y)), bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only fixture: a `tempfile::TempDir`-style holder using
    /// stdlib + a unique nanos suffix. Cleans up on drop.
    struct TempCacheDir(PathBuf);

    impl TempCacheDir {
        fn new() -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("ttymap-test-{}", nanos));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempCacheDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn url_uses_base_plus_zxy_pbf() {
        let fetcher = HttpFetcher::with_options("http://example.test".to_string(), None);
        assert_eq!(fetcher.base_url, "http://example.test");
        assert_eq!(fetcher.attribution(), ATTRIBUTION);
    }

    #[test]
    fn default_uses_mapscii_me_base_and_no_disk_cache() {
        let fetcher = HttpFetcher::new();
        assert_eq!(fetcher.base_url, "http://mapscii.me");
        assert!(fetcher.cache_dir.is_none());
    }

    #[test]
    fn read_disk_cache_returns_none_when_file_absent() {
        let dir = TempCacheDir::new();
        let key = TileKey::new(3, 1, 2);
        assert!(read_disk_cache(&dir.0, &key).is_none());
    }

    #[test]
    fn write_then_read_round_trips_bytes() {
        let dir = TempCacheDir::new();
        let key = TileKey::new(5, 17, 10);
        let payload = b"\x1f\x8b\x08fake gzip-ish payload";

        write_disk_cache(&dir.0, &key, payload);
        let got = read_disk_cache(&dir.0, &key).expect("disk hit");
        assert_eq!(got, payload);
    }

    #[test]
    fn disk_layout_uses_z_subdir_and_x_y_pbf_filename() {
        let dir = TempCacheDir::new();
        let key = TileKey::new(8, 42, 99);
        write_disk_cache(&dir.0, &key, b"x");
        assert!(dir.0.join("8").join("42-99.pbf").exists());
    }
}
