//! Tile cache — memory + disk storage with background HTTP fetching.
//! Owns the full tile lifecycle and all domain logic.
//! Interacts with TileClient only through its public API.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use directories::ProjectDirs;
use log::debug;

use super::client::TileClient;
use super::decode::{self, DecodedTile};

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
    client: TileClient,
    pending_count: Arc<AtomicUsize>,
    cache_dir: Option<PathBuf>,
    styler: Arc<crate::styler::Styler>,
    language: String,
    memory_cache: HashMap<TileKey, DecodedTile>,
    cache_order: VecDeque<TileKey>,
    cache_size: usize,
    current_z: u32,
    center_x: f64,
    center_y: f64,
    rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
}

impl TileCache {
    pub fn new(
        source_url: &str,
        enable_disk_cache: bool,
        styler: Arc<crate::styler::Styler>,
        language: String,
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

        let (tx, rx) = mpsc::channel();
        let client = TileClient::new(source_url, tx);
        let pending_count = Arc::new(AtomicUsize::new(0));

        TileCache {
            client,
            pending_count,
            cache_dir,
            styler,
            language,
            memory_cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_size: 64,
            current_z: 0,
            center_x: 0.0,
            center_y: 0.0,
            rx,
        }
    }

    pub fn pending_count(&self) -> Arc<AtomicUsize> {
        self.pending_count.clone()
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

        self.client
            .update_view(|key| key.z.abs_diff(cz) <= 1, &|key: &TileKey| {
                tile_distance_sq(key, cx, cy)
            });
        self.sync_pending_count();
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
            let decoded = decode::decode(&bytes, &self.styler, &self.language);

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
        self.sync_pending_count();
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
            let decoded = decode::decode(&bytes, &self.styler, &self.language);
            self.insert_memory(key.clone(), decoded);
            return self.memory_cache.get(&key);
        }

        let priority = tile_distance_sq(&key, self.center_x, self.center_y);
        self.client.enqueue(&key, priority);
        self.sync_pending_count();
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

        // z+1 center
        if z < 14 {
            let c = crate::geo::ll2tile(center_lon, center_lat, z + 1);
            let g = (1u64 << (z + 1)) as i32;
            let tx = (c.x.floor() as i32).rem_euclid(g);
            let ty = c.y.floor() as i32;
            if ty >= 0 && ty < g {
                self.get_tile(z + 1, tx, ty);
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

    fn sync_pending_count(&self) {
        self.pending_count
            .store(self.client.queue_len(), Ordering::Relaxed);
    }

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
}
