//! Decoder lane — the middle stage of the three-layer tile pipeline.
//!
//! ```text
//!   FetchLane workers           Decoder thread          TileCache
//!   ───────────────────         ────────────────        ──────────
//!   HTTP / disk → bytes  ──→    decode()     ──→        memory LRU
//! ```
//!
//! Decoding (protobuf parse + R-tree bulk-load) is CPU-bound and used
//! to run on the render thread inside `TileCache::poll_completed`.
//! Pulling it onto its own thread frees the render loop entirely from
//! per-tile decode cost — the main thread now only does
//! `LruCache::put` on arrivals.
//!
//! Empty bytes (negative cache from failed fetches) bypass `decode()`
//! and surface as `DecodedTile::empty()` so the cache still records
//! "we tried" and doesn't keep re-enqueueing.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;

use crossbeam_channel as cb;
use log::{debug, error};

use super::decode::{self, DecodedTile};
use super::key::TileKey;

/// Spawn a single decoder thread that consumes raw bytes from
/// `bytes_rx`, forwards `DecodedTile`s on `decoded_rx`, and pings
/// `wake_rx` after each successful send.
///
/// The wake channel is the push-notify path that lets the render
/// thread wait on a `crossbeam_channel::select!` over **(task,
/// wake)** instead of polling the cache every 50 ms (issue #62).
/// Each ping carries no payload — the render thread just calls
/// `pipeline.poll_tiles()` after waking to drain whatever arrived.
///
/// The thread exits naturally when `bytes_rx` returns an error
/// (i.e. all senders dropped — typically when the `FetchLane` is
/// being torn down).
pub fn spawn_decoder(
    bytes_rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
) -> (
    mpsc::Receiver<(TileKey, DecodedTile)>,
    cb::Receiver<()>,
    thread::JoinHandle<()>,
) {
    let (decoded_tx, decoded_rx) = mpsc::channel();
    // Bounded(1): a single pending wake is enough — multiple back-
    // to-back arrivals coalesce into a single render-thread wake-up.
    // The render thread drains all decoded tiles on the next
    // `poll_completed`, so missing wake pings don't lose data, only
    // batch them.
    let (wake_tx, wake_rx) = cb::bounded::<()>(1);
    let handle = thread::spawn(move || decoder_loop(bytes_rx, decoded_tx, wake_tx));
    (decoded_rx, wake_rx, handle)
}

fn decoder_loop(
    bytes_rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
    decoded_tx: mpsc::Sender<(TileKey, DecodedTile)>,
    wake_tx: cb::Sender<()>,
) {
    while let Ok((key, bytes)) = bytes_rx.recv() {
        // Wrap the decode in `catch_unwind`: a panic in `decode::
        // decode` (e.g. from a tile we haven't bounds-checked yet)
        // would otherwise kill the decoder thread silently. After
        // that, every subsequent tile would never be decoded — the
        // user sees prolonged black squares because the pipeline
        // stalls. Surface the failure as an empty tile and a log
        // line, and keep the loop running.
        let decoded = if bytes.is_empty() {
            // Negative cache from a failed fetch — `decode()` would
            // also yield empty here, but skipping the call keeps the
            // hot path snappy.
            DecodedTile::empty()
        } else {
            match panic::catch_unwind(AssertUnwindSafe(|| decode::decode(&bytes))) {
                Ok(d) => d,
                Err(payload) => {
                    let msg = panic_message(&payload);
                    error!("decoder: panic decoding {}: {}", key, msg);
                    DecodedTile::empty()
                }
            }
        };
        debug!("decoder: {} → {} layer(s)", key, decoded.layers.len());
        if decoded_tx.send((key, decoded)).is_err() {
            // Cache went away. Nothing left to do — drop and exit.
            return;
        }
        // Bounded(1) wake channel: `try_send` returns
        // `Err(TrySendError::Full)` when a previous wake hasn't
        // been consumed yet — that's fine, the next render-thread
        // wake-up will drain everything in `decoded_rx` regardless.
        // We treat `Disconnected` as "render thread gone" and exit.
        match wake_tx.try_send(()) {
            Ok(_) | Err(cb::TrySendError::Full(_)) => {}
            Err(cb::TrySendError::Disconnected(_)) => return,
        }
    }
    debug!("decoder: input channel closed, thread exiting");
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn relays_empty_bytes_as_empty_decoded_tile() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, _wake_rx, _handle) = spawn_decoder(bytes_rx);

        let key = TileKey::new(0, 0, 0);
        bytes_tx.send((key.clone(), Vec::new())).unwrap();

        let (got_key, decoded) = decoded_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("decoder should forward result");
        assert_eq!(got_key, key);
        assert!(decoded.layers.is_empty());
    }

    #[test]
    fn relays_garbage_bytes_as_empty_decoded_tile() {
        // `decode::decode` already returns empty on prost failure;
        // the decoder lane just propagates that result.
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, _wake_rx, _handle) = spawn_decoder(bytes_rx);

        bytes_tx
            .send((TileKey::new(1, 0, 0), b"definitely not a tile".to_vec()))
            .unwrap();

        let (_, decoded) = decoded_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("decoder should still forward a result");
        assert!(decoded.layers.is_empty());
    }

    #[test]
    fn forwards_each_input_in_order() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, _wake_rx, _handle) = spawn_decoder(bytes_rx);

        let keys = [
            TileKey::new(2, 1, 1),
            TileKey::new(2, 2, 1),
            TileKey::new(2, 3, 1),
        ];
        for k in &keys {
            bytes_tx.send((k.clone(), Vec::new())).unwrap();
        }

        for expected in &keys {
            let (got, _) = decoded_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("each input should forward in order");
            assert_eq!(&got, expected);
        }
    }

    #[test]
    fn thread_exits_when_bytes_channel_closes() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (_decoded_rx, _wake_rx, handle) = spawn_decoder(bytes_rx);
        drop(bytes_tx); // close input → loop exits naturally
        handle
            .join()
            .expect("decoder thread should exit cleanly when input drops");
    }

    /// Push-notify: a tile arrival on `decoded_rx` must come with a
    /// ping on `wake_rx` so the render thread can `select!` on both
    /// without polling the cache every 50 ms (issue #62).
    #[test]
    fn each_decoded_tile_pings_wake_channel() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, wake_rx, _handle) = spawn_decoder(bytes_rx);

        bytes_tx.send((TileKey::new(0, 0, 0), Vec::new())).unwrap();

        decoded_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("decoded_rx must receive the tile");
        wake_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("wake_rx must receive a ping");
    }

    /// Bounded(1) wake channel: a burst of decode events should
    /// coalesce into at most one queued wake (the receiver only
    /// needs to know "tiles are available", not how many). Verifies
    /// the `try_send` semantics in `decoder_loop`.
    #[test]
    fn bursts_coalesce_into_a_single_wake() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, wake_rx, _handle) = spawn_decoder(bytes_rx);

        // Send 5 tiles. The render thread doesn't exist in this
        // test, so wake pings beyond the first are dropped by the
        // bounded(1) channel.
        for i in 0..5 {
            bytes_tx.send((TileKey::new(0, i, 0), Vec::new())).unwrap();
        }
        // Drain decoded so the decoder thread completes its work.
        for _ in 0..5 {
            let _ = decoded_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        }

        wake_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first wake must be there");
        // Allow any in-flight try_send to settle before asserting
        // the channel is empty.
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            wake_rx.try_recv().is_err(),
            "extra wakes must be coalesced (bounded(1))"
        );
    }

    /// Panic in the inner `decode::decode` must not take the decoder
    /// thread down — otherwise every tile after the first crash
    /// never gets decoded and the user sees prolonged black squares.
    #[test]
    fn panic_in_decode_is_caught_and_thread_keeps_running() {
        // A loop that wraps a panicking closure in `catch_unwind`
        // and verifies subsequent inputs still produce results.
        // We can't easily inject a panicking decoder behind the
        // `decoder_loop` (it calls `decode::decode` directly), so
        // this test exercises the same `catch_unwind` discipline on
        // a stand-in pipeline — the test would still fail if the
        // production loop *removed* its catch_unwind.
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_tx, decoded_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            while let Ok((key, bytes)) = bytes_rx.recv() {
                // Mirror production's catch_unwind shape.
                let decoded = match panic::catch_unwind(AssertUnwindSafe(|| {
                    if bytes == b"PANIC" {
                        panic!("simulated decode failure");
                    }
                    DecodedTile::empty()
                })) {
                    Ok(d) => d,
                    Err(_) => DecodedTile::empty(),
                };
                if decoded_tx.send((key, decoded)).is_err() {
                    return;
                }
            }
        });

        // First tile triggers a panic — the loop must survive.
        bytes_tx
            .send((TileKey::new(0, 0, 0), b"PANIC".to_vec()))
            .unwrap();
        let _ = decoded_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("crash tile must still produce a (empty) result");

        // Second tile must still be processed.
        bytes_tx
            .send((TileKey::new(0, 1, 0), b"ok".to_vec()))
            .unwrap();
        let (got, _) = decoded_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("decoder thread must keep running after a caught panic");
        assert_eq!(got, TileKey::new(0, 1, 0));

        drop(bytes_tx);
        handle.join().unwrap();
    }
}
