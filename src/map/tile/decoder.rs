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

use std::sync::mpsc;
use std::thread;

use log::debug;

use super::decode::{self, DecodedTile};
use super::key::TileKey;

/// Spawn a single decoder thread that consumes raw bytes from
/// `bytes_rx` and forwards `DecodedTile`s on the returned channel.
///
/// The thread exits naturally when `bytes_rx` returns an error
/// (i.e. all senders dropped — typically when the `FetchLane` is
/// being torn down).
pub fn spawn_decoder(
    bytes_rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
) -> (
    mpsc::Receiver<(TileKey, DecodedTile)>,
    thread::JoinHandle<()>,
) {
    let (decoded_tx, decoded_rx) = mpsc::channel();
    let handle = thread::spawn(move || decoder_loop(bytes_rx, decoded_tx));
    (decoded_rx, handle)
}

fn decoder_loop(
    bytes_rx: mpsc::Receiver<(TileKey, Vec<u8>)>,
    decoded_tx: mpsc::Sender<(TileKey, DecodedTile)>,
) {
    while let Ok((key, bytes)) = bytes_rx.recv() {
        let decoded = if bytes.is_empty() {
            // Negative cache from a failed fetch — `decode()` would
            // also yield empty here, but skipping the call keeps the
            // hot path snappy.
            DecodedTile::empty()
        } else {
            decode::decode(&bytes)
        };
        debug!("decoder: {} → {} layer(s)", key, decoded.layers.len());
        if decoded_tx.send((key, decoded)).is_err() {
            // Cache went away. Nothing left to do — drop and exit.
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn relays_empty_bytes_as_empty_decoded_tile() {
        let (bytes_tx, bytes_rx) = mpsc::channel();
        let (decoded_rx, _handle) = spawn_decoder(bytes_rx);

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
        let (decoded_rx, _handle) = spawn_decoder(bytes_rx);

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
        let (decoded_rx, _handle) = spawn_decoder(bytes_rx);

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
        let (_decoded_rx, handle) = spawn_decoder(bytes_rx);
        drop(bytes_tx); // close input → loop exits naturally
        handle
            .join()
            .expect("decoder thread should exit cleanly when input drops");
    }
}
