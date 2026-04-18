//! HTTP MVT tile client — fixed worker pool.
//! Pops from internal queue, fetches via HTTP, returns raw bytes.
//! Cache interacts only through public methods, not internal state.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;

use log::debug;

use super::priority::TilePriority;
use super::queue::PriorityQueue;
use crate::tile::cache::TileKey;

const NUM_WORKERS: usize = 6;

struct SharedState {
    queue: Mutex<PriorityQueue<TileKey, TilePriority>>,
    condvar: Condvar,
    in_flight: Mutex<HashSet<TileKey>>,
    shutdown: AtomicBool,
}

pub struct HttpTileClient {
    shared: Arc<SharedState>,
    _workers: Vec<thread::JoinHandle<()>>,
}

impl HttpTileClient {
    pub fn new(source_url: &str, tx: mpsc::Sender<(TileKey, Vec<u8>)>) -> Self {
        let source_url = source_url.trim_end_matches('/').to_string();

        let shared = Arc::new(SharedState {
            queue: Mutex::new(PriorityQueue::new()),
            condvar: Condvar::new(),
            in_flight: Mutex::new(HashSet::new()),
            shutdown: AtomicBool::new(false),
        });

        // Build one reqwest Client and share it across workers. Client holds
        // the connection pool (HTTP keep-alive) and TLS context internally
        // via Arc, so Clone is cheap and the pool is shared.
        let http = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client build");

        let mut workers = Vec::with_capacity(NUM_WORKERS);
        for _ in 0..NUM_WORKERS {
            let shared = shared.clone();
            let source_url = source_url.clone();
            let tx = tx.clone();
            let http = http.clone();
            workers.push(thread::spawn(move || {
                worker_loop(&shared, &source_url, &tx, &http);
            }));
        }

        HttpTileClient {
            shared,
            _workers: workers,
        }
    }

    /// Enqueue a tile for fetching. Skips if already queued or in-flight.
    pub fn enqueue(&self, key: &TileKey, priority: TilePriority) {
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
    pub fn update_view<F>(&self, priority_fn: &F)
    where
        F: super::queue::PriorityFn<TileKey, TilePriority>,
    {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.reprioritize(priority_fn);
    }

    /// Number of tiles in queue + in-flight.
    pub fn pending_count(&self) -> usize {
        let queue = self.shared.queue.lock().unwrap();
        let in_flight = self.shared.in_flight.lock().unwrap();
        queue.len() + in_flight.len()
    }

    /// Number of tiles in queue only.
    pub fn queue_len(&self) -> usize {
        self.shared.queue.lock().unwrap().len()
    }

    /// Check if a key is currently being fetched.
    pub fn is_in_flight(&self, key: &TileKey) -> bool {
        self.shared.in_flight.lock().unwrap().contains(key)
    }
}

impl Drop for HttpTileClient {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.condvar.notify_all();
    }
}

// ── Worker ────────────────────────────────────────────────────────────────────

fn worker_loop(
    shared: &SharedState,
    source_url: &str,
    tx: &mpsc::Sender<(TileKey, Vec<u8>)>,
    http: &reqwest::blocking::Client,
) {
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
        let url = format!("{}/{}.pbf", source_url, key);
        debug!("worker: fetching {}", url);
        let bytes = fetch_http(http, &url);

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

fn fetch_http(client: &reqwest::blocking::Client, url: &str) -> Option<Vec<u8>> {
    let response = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            debug!("fetch error: {} - {}", url, e);
            return None;
        }
    };
    if !response.status().is_success() {
        debug!("fetch failed: {} - status {}", url, response.status());
        return None;
    }
    Some(response.bytes().ok()?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_dedup_in_flight() {
        let (tx, _rx) = mpsc::channel();
        let client = HttpTileClient::new("https://example.com", tx);
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
        assert_eq!(client.queue_len(), 0);
    }
}
