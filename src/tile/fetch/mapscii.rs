//! Client for `mapscii.me` — fetches MVT (`.pbf`) tiles over HTTP
//! using the slippy-map URL scheme `{base}/{z}/{x}/{y}.pbf`. The base
//! URL and attribution are hardcoded: ttymap's map rendering assumes
//! OSM-derived OpenMapTiles data, and mapscii.me is the only public
//! server that serves it without an API key. Fixed worker pool pops
//! from an internal queue, GETs bytes, and forwards the payload to
//! the cache through an `mpsc` channel.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;

use log::debug;

use super::TileClient;
use super::priority::TilePriority;
use super::queue::{PriorityFn, PriorityQueue};
use crate::shared::http::HttpClient;
use crate::tile::cache::TileKey;

const NUM_WORKERS: usize = 6;
const BASE_URL: &str = "http://mapscii.me";
const ATTRIBUTION: &str = "© OpenStreetMap contributors";

struct SharedState {
    queue: Mutex<PriorityQueue<TileKey, TilePriority>>,
    condvar: Condvar,
    in_flight: Mutex<HashSet<TileKey>>,
    shutdown: AtomicBool,
}

pub struct MapsciiTileClient {
    shared: Arc<SharedState>,
    _workers: Vec<thread::JoinHandle<()>>,
}

impl MapsciiTileClient {
    pub fn new(tx: mpsc::Sender<(TileKey, Vec<u8>)>) -> Self {
        let shared = Arc::new(SharedState {
            queue: Mutex::new(PriorityQueue::new()),
            condvar: Condvar::new(),
            in_flight: Mutex::new(HashSet::new()),
            shutdown: AtomicBool::new(false),
        });

        // One HttpClient shared across workers. Clone is cheap (reqwest's
        // internal Client is Arc-backed), so all workers reuse the same
        // connection pool. Tile servers are slow, hence the longer timeout.
        let http = HttpClient::with_timeout("tile", std::time::Duration::from_secs(10));

        let mut workers = Vec::with_capacity(NUM_WORKERS);
        for _ in 0..NUM_WORKERS {
            let shared = shared.clone();
            let tx = tx.clone();
            let http = http.clone();
            workers.push(thread::spawn(move || {
                worker_loop(&shared, &tx, &http);
            }));
        }

        MapsciiTileClient {
            shared,
            _workers: workers,
        }
    }
}

impl TileClient for MapsciiTileClient {
    /// Enqueue a tile for fetching. Skips if already queued or in-flight.
    fn enqueue(&self, key: &TileKey, priority: TilePriority) {
        {
            let in_flight = self.shared.in_flight.lock().unwrap();
            if in_flight.contains(key) {
                return;
            }
        }
        let mut queue = self.shared.queue.lock().unwrap();
        queue.push(key.clone(), priority);
        drop(queue);
        self.shared.condvar.notify_one();
    }

    /// Recompute queue priorities (typically after a viewport change).
    /// A `TilePriority` with a large `zoom_diff` sinks the entry to the
    /// back, where overflow drop will evict it as new work arrives.
    fn update_view(&self, priority_fn: &dyn PriorityFn<TileKey, TilePriority>) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.reprioritize(priority_fn);
    }

    fn attribution(&self) -> &str {
        ATTRIBUTION
    }
}

impl Drop for MapsciiTileClient {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.condvar.notify_all();
    }
}

// ── Worker ────────────────────────────────────────────────────────────────────

fn worker_loop(shared: &SharedState, tx: &mpsc::Sender<(TileKey, Vec<u8>)>, http: &HttpClient) {
    loop {
        let key = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(key) = queue.pop() {
                    let mut in_flight = shared.in_flight.lock().unwrap();
                    if in_flight.contains(&key) {
                        drop(in_flight);
                        continue;
                    }
                    in_flight.insert(key.clone());
                    break key;
                }
                queue = shared.condvar.wait(queue).unwrap();
            }
        };

        // HTTP fetch
        let url = format!("{}/{}.pbf", BASE_URL, key);
        debug!("worker: fetching {}", url);
        let bytes = http.get_bytes(&url);

        // Remove from in-flight
        shared.in_flight.lock().unwrap().remove(&key);

        // Send result (empty for failures → negative cache)
        let bytes = bytes.unwrap_or_default();
        debug!("worker: fetched {} ({} bytes)", key, bytes.len());
        if tx.send((key, bytes)).is_err() {
            log::warn!("tile channel closed");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_dedup_in_flight() {
        let (tx, _rx) = mpsc::channel();
        let client = MapsciiTileClient::new(tx);
        let key = TileKey::new(0, 0, 0);

        // Manually mark as in-flight
        client.shared.in_flight.lock().unwrap().insert(key.clone());

        // Should skip (already in-flight)
        client.enqueue(
            &key,
            TilePriority {
                zoom_diff: 0,
                distance_sq: 0.0,
            },
        );
        assert_eq!(client.shared.queue.lock().unwrap().len(), 0);
    }
}
