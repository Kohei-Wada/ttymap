//! Fire-and-poll background job.
//!
//! Our three async services (forward geocode, reverse geocode,
//! Wikipedia geosearch) all share the same shape: spawn a one-shot
//! thread that runs an HTTP call, push the result through an mpsc
//! channel, and let the UI thread drain completions with `try_recv`.
//! This module captures that shape so each service only owns its
//! client and request-shaping logic.

use std::sync::mpsc;
use std::thread;

pub struct AsyncJob<T> {
    tx: mpsc::Sender<T>,
    rx: mpsc::Receiver<T>,
}

impl<T: Send + 'static> AsyncJob<T> {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    /// Run `f` on a fresh background thread and send its result back
    /// to this job. The caller typically clones an `Arc<Client>` into
    /// `f` so the HTTP call survives the spawn.
    pub fn spawn<F>(&self, f: F)
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
    }

    /// Return one completed result, if any. Non-blocking.
    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

impl<T: Send + 'static> Default for AsyncJob<T> {
    fn default() -> Self {
        Self::new()
    }
}
