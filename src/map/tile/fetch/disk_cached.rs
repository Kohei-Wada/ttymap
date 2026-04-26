//! `DiskCachedFetcher<F>` — a decorator that adds an on-disk
//! read-through / write-through cache to any `TileFetcher`.
//!
//! Today the only inner is `HttpFetcher`, but the decorator pattern
//! means future backends (e.g. a remote PMTiles fetcher) get disk
//! cache for free. Backends that *are* themselves on disk (MBTiles)
//! don't get wrapped.
//!
//! Note: `TileCache` also reads the same on-disk layout as a
//! synchronous render-thread fast path (see `tile::disk`). The two
//! readers stay consistent because they share the helpers in
//! `tile::disk`.

use std::path::PathBuf;

use super::{FetchError, TileFetcher};
use crate::map::tile::disk;
use crate::map::tile::key::TileKey;

pub struct DiskCachedFetcher<F: TileFetcher> {
    inner: F,
    cache_dir: PathBuf,
}

impl<F: TileFetcher> DiskCachedFetcher<F> {
    pub fn new(inner: F, cache_dir: PathBuf) -> Self {
        Self { inner, cache_dir }
    }
}

impl<F: TileFetcher> TileFetcher for DiskCachedFetcher<F> {
    fn fetch(&self, key: &TileKey) -> Result<Vec<u8>, FetchError> {
        // Read-through.
        if let Some(bytes) = disk::read_disk(&self.cache_dir, key) {
            return Ok(bytes);
        }
        // Miss: ask the inner fetcher (HTTP, …).
        let bytes = self.inner.fetch(key)?;
        // Write-through. Best-effort; disk failures don't fail the
        // fetch — see `disk::write_disk`.
        disk::write_disk(&self.cache_dir, key, &bytes);
        Ok(bytes)
    }

    fn attribution(&self) -> &str {
        self.inner.attribution()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, fs};

    use super::*;

    /// Stdlib-only TempDir.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!("ttymap-disk-cached-{}", nanos));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Inner fetcher with a configurable canned response and an
    /// observable call counter.
    struct CannedFetcher {
        bytes: Vec<u8>,
        calls: Arc<AtomicUsize>,
        attribution: &'static str,
    }

    impl TileFetcher for CannedFetcher {
        fn fetch(&self, _key: &TileKey) -> Result<Vec<u8>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.bytes.clone())
        }
        fn attribution(&self) -> &str {
            self.attribution
        }
    }

    /// Inner fetcher that records every key it was asked to fetch.
    struct RecordingFetcher(Arc<Mutex<Vec<TileKey>>>);
    impl TileFetcher for RecordingFetcher {
        fn fetch(&self, key: &TileKey) -> Result<Vec<u8>, FetchError> {
            self.0.lock().unwrap().push(key.clone());
            Ok(b"fresh".to_vec())
        }
        fn attribution(&self) -> &str {
            ""
        }
    }

    #[test]
    fn disk_hit_short_circuits_inner_fetcher() {
        let dir = TempDir::new();
        let key = TileKey::new(3, 1, 2);
        // Plant a tile on disk.
        disk::write_disk(&dir.0, &key, b"on-disk");

        let calls = Arc::new(AtomicUsize::new(0));
        let inner = CannedFetcher {
            bytes: b"from-inner".to_vec(),
            calls: calls.clone(),
            attribution: "test",
        };
        let fetcher = DiskCachedFetcher::new(inner, dir.0.clone());

        let got = fetcher.fetch(&key).expect("disk hit");
        assert_eq!(got, b"on-disk");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "inner fetcher must not be called on disk hit"
        );
    }

    #[test]
    fn disk_miss_falls_through_and_writes_through() {
        let dir = TempDir::new();
        let key = TileKey::new(4, 5, 6);

        let calls = Arc::new(AtomicUsize::new(0));
        let inner = CannedFetcher {
            bytes: b"fresh".to_vec(),
            calls: calls.clone(),
            attribution: "test",
        };
        let fetcher = DiskCachedFetcher::new(inner, dir.0.clone());

        // First call: miss → inner fetcher → write-through.
        let got = fetcher.fetch(&key).expect("inner fetch");
        assert_eq!(got, b"fresh");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            disk::read_disk(&dir.0, &key),
            Some(b"fresh".to_vec()),
            "miss path must persist bytes for next time"
        );
    }

    #[test]
    fn second_fetch_after_write_through_is_a_disk_hit() {
        let dir = TempDir::new();
        let key = TileKey::new(2, 0, 0);

        let calls = Arc::new(AtomicUsize::new(0));
        let inner = CannedFetcher {
            bytes: b"once".to_vec(),
            calls: calls.clone(),
            attribution: "test",
        };
        let fetcher = DiskCachedFetcher::new(inner, dir.0.clone());

        let _ = fetcher.fetch(&key).unwrap();
        let _ = fetcher.fetch(&key).unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second fetch must be served from disk, not inner"
        );
    }

    #[test]
    fn attribution_delegates_to_inner() {
        let dir = TempDir::new();
        let inner = CannedFetcher {
            bytes: Vec::new(),
            calls: Arc::new(AtomicUsize::new(0)),
            attribution: "© Some Source",
        };
        let fetcher = DiskCachedFetcher::new(inner, dir.0.clone());
        assert_eq!(fetcher.attribution(), "© Some Source");
    }

    #[test]
    fn forwards_each_distinct_key_only_to_inner_when_uncached() {
        let dir = TempDir::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let fetcher = DiskCachedFetcher::new(RecordingFetcher(log.clone()), dir.0.clone());

        let keys = [
            TileKey::new(0, 0, 0),
            TileKey::new(0, 1, 0),
            TileKey::new(0, 0, 0),
        ];
        for k in &keys {
            let _ = fetcher.fetch(k);
        }

        let recorded = log.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![TileKey::new(0, 0, 0), TileKey::new(0, 1, 0)],
            "third call (repeat of first) must hit disk, not inner"
        );
    }
}
