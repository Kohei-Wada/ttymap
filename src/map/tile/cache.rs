//! Tile cache — memory + disk storage with background tile fetching.
//! Owns the full tile lifecycle and all domain logic.
//! Interacts with the fetch backend only through the `TileFetchLane` trait.

use std::collections::HashMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::mpsc;

use directories::ProjectDirs;
use log::debug;
use lru::LruCache;

use super::decode::{self, DecodedTile};
use super::fetch::{TileFetchLane, TilePriority};
use super::key::TileKey;

pub struct TileCache {
    client: Box<dyn TileFetchLane>,
    cache_dir: Option<PathBuf>,
    memory_cache: LruCache<TileKey, DecodedTile>,
    current_z: u32,
    center_x: f64,
    center_y: f64,
    rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
}

impl TileCache {
    /// Build a cache around an injected `TileFetchLane`. The `rx`
    /// channel is the receiving end of the pair whose `tx` was handed
    /// to the lane — completed fetches arrive on it.
    ///
    /// Wiring (channel + lane construction) lives at the composition
    /// root in `app::build_tile_cache`, where backend selection is made.
    pub fn new(
        client: Box<dyn TileFetchLane>,
        rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
        enable_disk_cache: bool,
        cache_size: usize,
    ) -> Self {
        let cache_dir = if enable_disk_cache {
            ProjectDirs::from("", "", "ttymap").map(|proj_dirs| {
                let dir = proj_dirs.cache_dir().to_path_buf();
                let _ = fs::create_dir_all(&dir);
                dir
            })
        } else {
            None
        };

        // `cache_size` may legitimately be configured to 0 by paranoid
        // users; clamp to 1 since `NonZeroUsize::new` rejects zero.
        let capacity = NonZeroUsize::new(cache_size.max(1)).unwrap();
        TileCache {
            client,
            cache_dir,
            memory_cache: LruCache::new(capacity),
            current_z: 0,
            center_x: 0.0,
            center_y: 0.0,
            rx,
        }
    }

    /// Update view state. Purges stale queue entries and re-sorts by distance.
    pub fn set_view(&mut self, center_lon: f64, center_lat: f64, z: u32) {
        self.current_z = z;
        let center = crate::geo::ll2tile(center_lon, center_lat, z);
        self.center_x = center.x;
        self.center_y = center.y;

        let cx = self.center_x;
        let cy = self.center_y;
        let cz = self.current_z;
        // Modular distance is computed in `cz`-space (matching cx/cy)
        // so the antimeridian wrap is correct for same-z keys. Cross-z
        // scoring is approximate but `zoom_diff` already dominates the
        // composite priority.
        let grid_size = 1i32.checked_shl(cz).unwrap_or(i32::MAX);

        self.client.update_view(&|key: &TileKey| TilePriority {
            zoom_diff: key.z.abs_diff(cz),
            distance_sq: tile_distance_sq(key, cx, cy, grid_size),
        });
    }

    /// Whether the fetch backend has finished all outstanding work.
    /// `true` means the queue is empty and no tiles are in-flight —
    /// there will be no more tile arrivals without a new request.
    pub fn is_fetch_idle(&self) -> bool {
        self.client.is_idle()
    }

    /// Drain completed HTTP fetches: decode, save to disk, insert to memory.
    pub fn poll_completed(&mut self) -> bool {
        let mut any_new = false;
        while let Ok((key, bytes)) = self.rx.try_recv() {
            let is_current = key.z.abs_diff(self.current_z) <= 1;

            if bytes.is_empty() {
                debug!("poll_completed: negative cache for {}", key);
                let empty = DecodedTile {
                    layers: HashMap::new(),
                };
                self.insert_memory(key, empty);
                continue;
            }

            self.write_disk_cache(&key, &bytes);
            let decoded = decode::decode(&bytes);

            if is_current {
                debug!(
                    "poll_completed: decoded tile {} ({} layers)",
                    key,
                    decoded.layers.len()
                );
                any_new = true;
            }
            self.insert_memory(key, decoded);
        }
        any_new
    }

    /// Get a tile. Checks: memory → disk → enqueue HTTP fetch.
    ///
    /// On a memory hit, `LruCache::get` promotes the key to MRU so a
    /// hot tile can survive a long pan across many cold tiles (issue
    /// #105).
    pub fn get_tile(&mut self, z: u32, x: i32, y: i32) -> Option<&DecodedTile> {
        let key = TileKey::new(z, x, y);

        if self.memory_cache.contains(&key) {
            return self.memory_cache.get(&key);
        }

        if let Some(bytes) = self.read_disk_cache(&key) {
            debug!("disk cache hit: {} ({} bytes)", key, bytes.len());
            let decoded = decode::decode(&bytes);
            self.insert_memory(key.clone(), decoded);
            return self.memory_cache.get(&key);
        }

        let grid_size = 1i32.checked_shl(self.current_z).unwrap_or(i32::MAX);
        let priority = TilePriority {
            zoom_diff: key.z.abs_diff(self.current_z),
            distance_sq: tile_distance_sq(&key, self.center_x, self.center_y, grid_size),
        };
        self.client.enqueue(&key, priority);
        None
    }

    /// Prefetch nearby tiles for smoother panning/zooming.
    pub fn prefetch(&mut self, center_lon: f64, center_lat: f64, zoom: f64) {
        let z = crate::geo::base_zoom(zoom);
        let center = crate::geo::ll2tile(center_lon, center_lat, z);
        let grid_size = (1u64 << z) as i32;
        let cx = center.x.floor() as i32;
        let cy = center.y.floor() as i32;

        // 1-tile border (no corners)
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                if (-1..=1).contains(&dx) && (-1..=1).contains(&dy) {
                    continue;
                }
                if dx.abs() == 2 && dy.abs() == 2 {
                    continue;
                }
                let ty = cy + dy;
                if ty < 0 || ty >= grid_size {
                    continue;
                }
                let tx = (cx + dx).rem_euclid(grid_size);
                self.get_tile(z, tx, ty);
            }
        }

        // z+1: all 4 children of the center tile, so a zoom-in lands on
        // already-warm tiles regardless of which quadrant the view
        // fractionally sits on.
        if z < 14 {
            let g = (1u64 << (z + 1)) as i32;
            let base_x = cx * 2;
            let base_y = cy * 2;
            for dy in 0..2 {
                for dx in 0..2 {
                    let ty = base_y + dy;
                    if ty < 0 || ty >= g {
                        continue;
                    }
                    let tx = (base_x + dx).rem_euclid(g);
                    self.get_tile(z + 1, tx, ty);
                }
            }
        }

        // z-1 center
        if z > 0 {
            let c = crate::geo::ll2tile(center_lon, center_lat, z - 1);
            let g = (1u64 << (z - 1)) as i32;
            let tx = (c.x.floor() as i32).rem_euclid(g);
            let ty = c.y.floor() as i32;
            if ty >= 0 && ty < g {
                self.get_tile(z - 1, tx, ty);
            }
        }
    }

    // ── Private ───────────────────────────────────────────────────────────

    fn read_disk_cache(&self, key: &TileKey) -> Option<Vec<u8>> {
        let dir = self.cache_dir.as_ref()?;
        fs::read(
            dir.join(key.z.to_string())
                .join(format!("{}-{}.pbf", key.x, key.y)),
        )
        .ok()
    }

    fn write_disk_cache(&self, key: &TileKey, bytes: &[u8]) {
        if let Some(dir) = &self.cache_dir {
            let tile_dir = dir.join(key.z.to_string());
            let _ = fs::create_dir_all(&tile_dir);
            let _ = fs::write(tile_dir.join(format!("{}-{}.pbf", key.x, key.y)), bytes);
        }
    }

    fn insert_memory(&mut self, key: TileKey, decoded: DecodedTile) {
        // `LruCache::put` evicts the least-recently-used entry when at
        // capacity and treats the inserted key as MRU.
        self.memory_cache.put(key, decoded);
    }
}

/// Distance² from tile center to view center (for priority sorting).
///
/// X is modular: when the view straddles the antimeridian, a tile on
/// the wrap side is geographically adjacent and must score that way
/// (issue #106). `grid_size` is `1 << z` for the relevant zoom.
/// Y is **not** modular — slippy maps have no polar wrap.
fn tile_distance_sq(key: &TileKey, center_x: f64, center_y: f64, grid_size: i32) -> f64 {
    let raw_dx = key.x as f64 + 0.5 - center_x;
    let g = grid_size as f64;
    let dx = if g > 0.0 && raw_dx.abs() > g / 2.0 {
        raw_dx - raw_dx.signum() * g
    } else {
        raw_dx
    };
    let dy = key.y as f64 + 0.5 - center_y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::fetch::queue::PriorityFn;
    use super::*;

    #[test]
    fn test_tile_distance_sq() {
        // Same hemisphere, centred on the tile.
        let d = tile_distance_sq(&TileKey::new(0, 5, 5), 5.5, 5.5, 1024);
        assert!(d < 0.01);
    }

    /// Regression for issue #106. Near the antimeridian, a tile that
    /// is geographically adjacent on the wrap side has a tiny
    /// modular distance — but with a naked `key.x - center_x`
    /// subtract it scored as `~grid_size²` and the priority queue
    /// dropped it. Distance must wrap on the x axis.
    #[test]
    fn tile_distance_sq_wraps_x_at_antimeridian() {
        // grid_size = 8 (z=3). View centred on the right edge.
        // center_x = 7.5 → on tile (7, _). Key (0, _) is the
        // wrapped-around tile across the date line.
        let d = tile_distance_sq(&TileKey::new(3, 0, 5), 7.5, 5.5, 8);
        assert!(d < 2.0, "wrap-side tile must score as ~adjacent (got {d})");
    }

    #[test]
    fn tile_distance_sq_wraps_x_other_direction() {
        // Mirror: center near left edge, key on the right edge.
        // center_x=0.5, key.x=7, grid_size=8 → wrapped distance is 1.
        let d = tile_distance_sq(&TileKey::new(3, 7, 5), 0.5, 5.5, 8);
        assert!(d < 2.0, "wrap on the other side too (got {d})");
    }

    #[test]
    fn tile_distance_sq_does_not_wrap_when_direct_path_is_shorter() {
        // Center mid-grid, key at left edge — direct path (3.5) is
        // shorter than wrap (4.5). No wrap.
        // raw_dx = 0.5 - 4 = -3.5; |raw| = 3.5 <= grid/2 = 4 → keep.
        let d = tile_distance_sq(&TileKey::new(3, 0, 5), 4.0, 5.0, 8);
        let expected = 3.5_f64.powi(2) + 0.5_f64.powi(2);
        assert!((d - expected).abs() < 0.01, "expected {expected}, got {d}");
    }

    /// Y axis is **not** modular — no polar wrap exists for slippy-
    /// map tiles, so a key with a wildly different y must keep its
    /// large distance even if x is close.
    #[test]
    fn tile_distance_sq_does_not_wrap_y() {
        // grid_size large enough to avoid any x wrap. Difference is
        // pure y.
        let d = tile_distance_sq(&TileKey::new(3, 5, 0), 5.5, 7.5, 8);
        assert!(d > 40.0, "y must not wrap (got {d})");
    }

    /// A no-op `TileFetchLane` that swallows everything. Suitable
    /// for tests that exercise cache state without checking dispatch.
    struct NoopLane;
    impl TileFetchLane for NoopLane {
        fn enqueue(&self, _: &TileKey, _: TilePriority) {}
        fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
        fn attribution(&self) -> &str {
            "noop"
        }
        fn is_idle(&self) -> bool {
            true
        }
    }

    /// Regression for issue #105. The doc comment / data-structure
    /// names promise LRU eviction, but the implementation used FIFO
    /// (insertion order) — a hot tile that was hit between inserts
    /// got evicted at the same time as cold ones. Under true LRU,
    /// recent access promotes the tile to MRU and protects it.
    #[test]
    fn lru_eviction_keeps_recently_accessed_tile() {
        // cache_size = 2. Inject A, B via empty-bytes negative-cache
        // path (poll_completed inserts an empty DecodedTile). Hit A
        // (which under LRU bumps it to MRU). Insert C. With LRU, B
        // (LRU) is evicted; A survives. With FIFO, A (oldest insert)
        // is evicted regardless of the hit.
        let (tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(Box::new(NoopLane), rx, false, 2);

        let a = TileKey::new(0, 0, 0);
        let b = TileKey::new(0, 1, 0);
        let c = TileKey::new(0, 2, 0);

        tx.send((a.clone(), Vec::new())).unwrap();
        tx.send((b.clone(), Vec::new())).unwrap();
        cache.poll_completed();

        // Access A: bumps to MRU under LRU.
        assert!(cache.get_tile(a.z, a.x, a.y).is_some());

        tx.send((c.clone(), Vec::new())).unwrap();
        cache.poll_completed();

        assert!(
            cache.get_tile(a.z, a.x, a.y).is_some(),
            "recently-accessed tile A must survive when C displaces \
             the LRU entry (B), not the most-recently-touched one"
        );
    }

    /// Proves `TileCache` drives its backend purely through the
    /// `TileFetchLane` trait: cache misses dispatch to the injected
    /// lane, with no HTTP or worker threads involved.
    #[test]
    fn test_cache_misses_dispatch_through_injected_client() {
        struct RecordingLane(Arc<Mutex<Vec<TileKey>>>);
        impl TileFetchLane for RecordingLane {
            fn enqueue(&self, key: &TileKey, _: TilePriority) {
                self.0.lock().unwrap().push(key.clone());
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                "mock"
            }
            fn is_idle(&self) -> bool {
                true
            }
        }

        let log = Arc::new(Mutex::new(Vec::<TileKey>::new()));
        let client: Box<dyn TileFetchLane> = Box::new(RecordingLane(log.clone()));
        let (_tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(client, rx, false, 64);

        cache.get_tile(3, 1, 2);
        cache.get_tile(3, 5, 6);

        assert_eq!(
            *log.lock().unwrap(),
            vec![TileKey::new(3, 1, 2), TileKey::new(3, 5, 6)],
        );
    }
}
