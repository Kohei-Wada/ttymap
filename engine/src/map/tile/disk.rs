//! On-disk tile cache layout + read/write primitives.
//!
//! Two consumers share these helpers:
//!
//! - [`fetch::DiskCachedFetcher`] writes through on every successful
//!   inner-fetch and short-circuits on hit when called from the
//!   worker pool.
//! - [`cache::TileCache`] uses the read side as a synchronous
//!   render-thread fast path, sending hit-bytes straight to the
//!   decoder lane and bypassing the worker queue. Skipping the
//!   worker queue matters under fast zoom where overflow drop would
//!   otherwise discard legitimate disk-resident tiles.
//!
//! Layout (unchanged from the pre-refactor scheme so existing user
//! caches keep working):
//!
//! ```text
//!   {cache_dir}/{z}/{x}-{y}.pbf
//! ```

use std::fs;
use std::path::Path;

use super::key::TileKey;

/// Read a tile's bytes from disk. `None` if the file is absent or
/// can't be opened — disk failures are non-fatal and fall through to
/// the next layer.
pub fn read_disk(dir: &Path, key: &TileKey) -> Option<Vec<u8>> {
    fs::read(
        dir.join(key.z.to_string())
            .join(format!("{}-{}.pbf", key.x, key.y)),
    )
    .ok()
}

/// Write a tile's bytes to disk. Best-effort: a failure to create
/// the per-zoom subdirectory or write the file is silently dropped
/// (the in-memory cache will still see this tile, and the next
/// process will just refetch from origin).
pub fn write_disk(dir: &Path, key: &TileKey, bytes: &[u8]) {
    let tile_dir = dir.join(key.z.to_string());
    let _ = fs::create_dir_all(&tile_dir);
    let _ = fs::write(tile_dir.join(format!("{}-{}.pbf", key.x, key.y)), bytes);
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    /// Stdlib-only TempDir-style holder using a unique nanos suffix.
    /// Cleans up on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("ttymap-{}-{}", label, nanos));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn read_returns_none_when_file_absent() {
        let dir = TempDir::new("disk-absent");
        let key = TileKey::new(3, 1, 2);
        assert!(read_disk(&dir.0, &key).is_none());
    }

    #[test]
    fn write_then_read_round_trips_bytes() {
        let dir = TempDir::new("disk-roundtrip");
        let key = TileKey::new(5, 17, 10);
        let payload = b"\x1f\x8b\x08fake gzip-ish payload";

        write_disk(&dir.0, &key, payload);
        let got = read_disk(&dir.0, &key).expect("disk hit");
        assert_eq!(got, payload);
    }

    #[test]
    fn layout_uses_z_subdir_and_x_y_pbf_filename() {
        let dir = TempDir::new("disk-layout");
        let key = TileKey::new(8, 42, 99);
        write_disk(&dir.0, &key, b"x");
        assert!(dir.0.join("8").join("42-99.pbf").exists());
    }

    #[test]
    fn read_returns_none_for_unrelated_zoom_directory() {
        // Planted at z=4 but reading at z=5 must miss.
        let dir = TempDir::new("disk-zoom-iso");
        write_disk(&dir.0, &TileKey::new(4, 1, 1), b"x");
        assert!(read_disk(&dir.0, &TileKey::new(5, 1, 1)).is_none());
    }
}
