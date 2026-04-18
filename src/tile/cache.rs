//! Tile cache — memory + disk storage with background tile fetching.
//! Owns the full tile lifecycle and all domain logic.
//! Interacts with the fetch backend only through the `TileClient` trait.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;

use directories::ProjectDirs;
use log::debug;

use super::decode::{self, DecodedTile};
use super::fetch::{TileClient, TilePriority};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub z: u32,
    pub x: i32,
    pub y: i32,
}

impl TileKey {
    pub fn new(z: u32, x: i32, y: i32) -> Self {
        Self { z, x, y }
    }
}

impl std::fmt::Display for TileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.z, self.x, self.y)
    }
}

pub struct TileCache {
    client: Box<dyn TileClient>,
    cache_dir: Option<PathBuf>,
    memory_cache: HashMap<TileKey, DecodedTile>,
    cache_order: VecDeque<TileKey>,
    cache_size: usize,
    current_z: u32,
    center_x: f64,
    center_y: f64,
    rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
}

impl TileCache {
    /// Build a cache around an injected `TileClient`. The `rx` channel
    /// is the receiving end of the pair whose `tx` was handed to the
    /// client — completed fetches arrive on it.
    ///
    /// Most callers should use `crate::tile::build_tile_cache`, which
    /// wires the channel and `select_client` together.
    pub fn new(
        client: Box<dyn TileClient>,
        rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
        enable_disk_cache: bool,
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

        TileCache {
            client,
            cache_dir,
            memory_cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_size: 64,
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

        self.client.update_view(&|key: &TileKey| TilePriority {
            zoom_diff: key.z.abs_diff(cz),
            distance_sq: tile_distance_sq(key, cx, cy),
        });
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
    pub fn get_tile(&mut self, z: u32, x: i32, y: i32) -> Option<&DecodedTile> {
        let key = TileKey::new(z, x, y);

        if self.memory_cache.contains_key(&key) {
            return self.memory_cache.get(&key);
        }

        if let Some(bytes) = self.read_disk_cache(&key) {
            debug!("disk cache hit: {} ({} bytes)", key, bytes.len());
            let decoded = decode::decode(&bytes);
            self.insert_memory(key.clone(), decoded);
            return self.memory_cache.get(&key);
        }

        let priority = TilePriority {
            zoom_diff: key.z.abs_diff(self.current_z),
            distance_sq: tile_distance_sq(&key, self.center_x, self.center_y),
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
        if self.cache_order.len() >= self.cache_size
            && let Some(oldest) = self.cache_order.pop_front()
        {
            self.memory_cache.remove(&oldest);
        }
        self.memory_cache.insert(key.clone(), decoded);
        self.cache_order.push_back(key);
    }
}

/// Distance² from tile center to view center (for priority sorting).
fn tile_distance_sq(key: &TileKey, center_x: f64, center_y: f64) -> f64 {
    let dx = key.x as f64 + 0.5 - center_x;
    let dy = key.y as f64 + 0.5 - center_y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::super::fetch::queue::PriorityFn;
    use super::*;

    #[test]
    fn test_tile_key_display() {
        assert_eq!(TileKey::new(5, 17, 10).to_string(), "5/17/10");
    }

    #[test]
    fn test_tile_distance_sq() {
        let d = tile_distance_sq(&TileKey::new(0, 5, 5), 5.5, 5.5);
        assert!(d < 0.01);
    }

    /// Proves `TileCache` drives its backend purely through the `TileClient`
    /// trait: cache misses dispatch to the injected client, with no HTTP
    /// or worker threads involved.
    #[test]
    fn test_cache_misses_dispatch_through_injected_client() {
        struct RecordingClient(Arc<Mutex<Vec<TileKey>>>);
        impl TileClient for RecordingClient {
            fn enqueue(&self, key: &TileKey, _: TilePriority) {
                self.0.lock().unwrap().push(key.clone());
            }
            fn update_view(&self, _: &dyn PriorityFn<TileKey, TilePriority>) {}
            fn attribution(&self) -> &str {
                "mock"
            }
        }

        let log = Arc::new(Mutex::new(Vec::<TileKey>::new()));
        let client: Box<dyn TileClient> = Box::new(RecordingClient(log.clone()));
        let (_tx, rx) = mpsc::channel();
        let mut cache = TileCache::new(client, rx, false);

        cache.get_tile(3, 1, 2);
        cache.get_tile(3, 5, 6);

        assert_eq!(
            *log.lock().unwrap(),
            vec![TileKey::new(3, 1, 2), TileKey::new(3, 5, 6)],
        );
    }
}
