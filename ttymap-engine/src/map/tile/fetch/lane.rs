//! Generic per-backend fetch lane: owns the priority queue, the
//! in-flight set, and a fixed-size worker pool. Each worker pops a
//! key, calls the wrapped [`TileFetcher`]'s `fetch`, and forwards the
//! resulting bytes to the cache through an `mpsc` channel.
//!
//! By keeping queue / worker / dedup logic here, every concrete
//! backend (HTTP, mbtiles, pmtiles, …) only writes its `TileFetcher`
//! impl and is automatically wired up with priority queueing,
//! reprioritize-on-view-change, in-flight dedup, and overflow drop.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::thread;

use log::debug;

use super::priority::TilePriority;
use super::queue::{PriorityFn, PriorityQueue};
use super::{TileFetchLane, TileFetcher};
use crate::map::tile::key::TileKey;

struct SharedState {
    queue: Mutex<PriorityQueue<TileKey, TilePriority>>,
    condvar: Condvar,
    in_flight: Mutex<HashSet<TileKey>>,
    shutdown: AtomicBool,
}

impl SharedState {
    // Poison = a worker panicked mid-update; crash rather than risk
    // serving corrupted queue state ("Mutex poison = process death"
    // in docs/design.md).
    fn lock_queue(&self) -> MutexGuard<'_, PriorityQueue<TileKey, TilePriority>> {
        self.queue.lock().expect("tile worker mutex poisoned")
    }

    fn lock_in_flight(&self) -> MutexGuard<'_, HashSet<TileKey>> {
        self.in_flight.lock().expect("tile worker mutex poisoned")
    }
}

pub struct FetchLane<F: TileFetcher> {
    fetcher: Arc<F>,
    shared: Arc<SharedState>,
    _workers: Vec<thread::JoinHandle<()>>,
}

impl<F: TileFetcher + 'static> FetchLane<F> {
    /// Wrap `fetcher` in a `num_workers`-sized worker pool. Each
    /// worker forwards completed fetches as `(key, bytes)` to `tx`;
    /// failures arrive as `(key, Vec::new())` so the cache can
    /// negative-cache them.
    pub fn new(fetcher: F, num_workers: usize, tx: mpsc::Sender<(TileKey, Vec<u8>)>) -> Self {
        let shared = Arc::new(SharedState {
            queue: Mutex::new(PriorityQueue::new()),
            condvar: Condvar::new(),
            in_flight: Mutex::new(HashSet::new()),
            shutdown: AtomicBool::new(false),
        });
        let fetcher = Arc::new(fetcher);

        let mut workers = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let shared = shared.clone();
            let tx = tx.clone();
            let fetcher = fetcher.clone();
            workers.push(thread::spawn(move || {
                worker_loop(&shared, &tx, &*fetcher);
            }));
        }

        Self {
            fetcher,
            shared,
            _workers: workers,
        }
    }
}

impl<F: TileFetcher + 'static> TileFetchLane for FetchLane<F> {
    fn enqueue(&self, key: &TileKey, priority: TilePriority) {
        {
            let in_flight = self.shared.lock_in_flight();
            if in_flight.contains(key) {
                return;
            }
        }
        let mut queue = self.shared.lock_queue();
        queue.push(key.clone(), priority);
        drop(queue);
        self.shared.condvar.notify_one();
    }

    /// Recompute queue priorities. A `TilePriority` with a large
    /// `zoom_diff` sinks the entry to the back, where overflow drop
    /// will evict it as new work arrives.
    fn update_view(&self, priority_fn: &dyn PriorityFn<TileKey, TilePriority>) {
        let mut queue = self.shared.lock_queue();
        queue.reprioritize(priority_fn);
    }

    fn attribution(&self) -> &str {
        self.fetcher.attribution()
    }

    fn is_idle(&self) -> bool {
        let queue = self.shared.lock_queue();
        let in_flight = self.shared.lock_in_flight();
        queue.is_empty() && in_flight.is_empty()
    }
}

impl<F: TileFetcher> Drop for FetchLane<F> {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        self.shared.condvar.notify_all();
    }
}

// ── Worker ────────────────────────────────────────────────────────────────────

fn worker_loop<F: TileFetcher + ?Sized>(
    shared: &SharedState,
    tx: &mpsc::Sender<(TileKey, Vec<u8>)>,
    fetcher: &F,
) {
    loop {
        let key = {
            let mut queue = shared.lock_queue();
            loop {
                if shared.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(key) = queue.pop() {
                    let mut in_flight = shared.lock_in_flight();
                    if in_flight.contains(&key) {
                        drop(in_flight);
                        continue;
                    }
                    in_flight.insert(key.clone());
                    break key;
                }
                queue = shared
                    .condvar
                    .wait(queue)
                    .expect("tile worker condvar poisoned");
            }
        };

        debug!("worker: fetching {}", key);
        let bytes = match fetcher.fetch(&key) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("tile: fetch failed for {}: {}", key, e);
                Vec::new()
            }
        };

        // Remove from in-flight before sending so a re-enqueue racing
        // with `tx.send` doesn't dedup-skip on a stale entry.
        shared.lock_in_flight().remove(&key);

        debug!("worker: fetched {} ({} bytes)", key, bytes.len());
        if tx.send((key, bytes)).is_err() {
            log::warn!("tile channel closed");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::FetchError;
    use super::*;

    /// Echoes the key into the result bytes so a test can verify
    /// round-trip without protobuf.
    struct EchoFetcher {
        attribution: &'static str,
    }

    impl TileFetcher for EchoFetcher {
        fn fetch(&self, key: &TileKey) -> Result<Vec<u8>, FetchError> {
            Ok(format!("{}", key).into_bytes())
        }
        fn attribution(&self) -> &str {
            self.attribution
        }
    }

    /// Fetcher that always errors — exercises the negative-cache path.
    struct FailingFetcher;
    impl TileFetcher for FailingFetcher {
        fn fetch(&self, _key: &TileKey) -> Result<Vec<u8>, FetchError> {
            Err(FetchError::new("nope"))
        }
        fn attribution(&self) -> &str {
            ""
        }
    }

    fn p() -> TilePriority {
        TilePriority {
            zoom_diff: 0,
            distance_sq: 0.0,
        }
    }

    #[test]
    fn enqueue_dispatches_to_fetcher_and_forwards_bytes() {
        let (tx, rx) = mpsc::channel();
        let lane = FetchLane::new(
            EchoFetcher {
                attribution: "test",
            },
            1,
            tx,
        );
        lane.enqueue(&TileKey::new(2, 3, 4), p());

        let (key, bytes) = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker should forward result");
        assert_eq!(key, TileKey::new(2, 3, 4));
        assert_eq!(bytes, b"2/3/4");
    }

    #[test]
    fn enqueue_skips_when_already_in_flight() {
        let (tx, _rx) = mpsc::channel();
        let lane = FetchLane::new(
            EchoFetcher {
                attribution: "test",
            },
            // Zero workers so the in-flight entry we plant doesn't
            // get drained out from under the assertion.
            0,
            tx,
        );
        let key = TileKey::new(0, 0, 0);

        lane.shared.lock_in_flight().insert(key.clone());

        lane.enqueue(&key, p());

        let queued = lane.shared.lock_queue().len();
        assert_eq!(queued, 0, "in-flight key must not be re-enqueued");
    }

    #[test]
    fn fetch_error_yields_empty_bytes_on_channel() {
        let (tx, rx) = mpsc::channel();
        let lane = FetchLane::new(FailingFetcher, 1, tx);
        lane.enqueue(&TileKey::new(0, 0, 0), p());
        let (_, bytes) = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker should forward empty result on error");
        assert!(
            bytes.is_empty(),
            "fetch failure must surface as empty bytes (negative cache)"
        );
    }

    #[test]
    fn lane_attribution_delegates_to_fetcher() {
        let (tx, _rx) = mpsc::channel();
        let lane = FetchLane::new(
            EchoFetcher {
                attribution: "© Some Source",
            },
            // 0 workers — we don't need to drain.
            0,
            tx,
        );
        assert_eq!(
            <FetchLane<EchoFetcher> as TileFetchLane>::attribution(&lane),
            "© Some Source"
        );
    }

    #[test]
    fn is_idle_reflects_queue_and_in_flight_state() {
        let (tx, _rx) = mpsc::channel();
        let lane = FetchLane::new(FailingFetcher, 0, tx);
        assert!(<FetchLane<FailingFetcher> as TileFetchLane>::is_idle(&lane));

        lane.shared.lock_queue().push(TileKey::new(0, 0, 0), p());
        assert!(!<FetchLane<FailingFetcher> as TileFetchLane>::is_idle(
            &lane
        ));
    }
}
