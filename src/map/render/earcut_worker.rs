//! Hard-timeout wrapper for `earcut` polygon triangulation.
//!
//! `earcut` can hang (not panic) on degenerate / self-intersecting input
//! that survives our Sutherland–Hodgman clip. `silence_panics` catches
//! panics; this catches infinite loops by running earcut on a dedicated
//! thread and giving up after a deadline. The hung worker becomes a
//! zombie (we cannot interrupt it from another thread in safe Rust); a
//! fresh worker takes its place. Zombies eventually exit when their
//! channels disconnect after earcut finally returns — or never, if the
//! loop is truly infinite.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

struct Req {
    verts: Vec<[f64; 2]>,
    holes: Vec<usize>,
}

pub struct EarcutWorker {
    req_tx: mpsc::Sender<Req>,
    resp_rx: mpsc::Receiver<Vec<usize>>,
}

impl EarcutWorker {
    pub fn new() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<Req>();
        let (resp_tx, resp_rx) = mpsc::channel::<Vec<usize>>();
        thread::spawn(move || {
            let mut earcut = earcut::Earcut::<f64>::new();
            let mut out: Vec<usize> = Vec::new();
            while let Ok(req) = req_rx.recv() {
                out.clear();
                let _ = super::panic_silence::silence_panics(|| {
                    earcut.earcut(req.verts.iter().copied(), &req.holes, &mut out);
                });
                if resp_tx.send(std::mem::take(&mut out)).is_err() {
                    return;
                }
            }
        });
        Self { req_tx, resp_rx }
    }

    /// Triangulate, giving up after `timeout`. Returns the bare indices
    /// on success or a typed error so the caller can attach polygon
    /// context to a warn-level log on `TimedOut`. On timeout the worker
    /// is replaced; the old one keeps running until earcut returns (or
    /// forever).
    pub fn triangulate(
        &mut self,
        verts: Vec<[f64; 2]>,
        holes: Vec<usize>,
        timeout: Duration,
    ) -> Result<Vec<usize>, TriangulateError> {
        if self.req_tx.send(Req { verts, holes }).is_err() {
            // Worker died (panic in our framing code, not earcut). Restart.
            *self = Self::new();
            return Err(TriangulateError::WorkerDied);
        }
        match self.resp_rx.recv_timeout(timeout) {
            Ok(indices) => Ok(indices),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                *self = Self::new();
                Err(TriangulateError::TimedOut)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                *self = Self::new();
                Err(TriangulateError::WorkerDied)
            }
        }
    }
}

#[derive(Debug)]
pub enum TriangulateError {
    /// earcut did not finish within the deadline; old worker abandoned.
    TimedOut,
    /// Worker thread is gone (panic in framing code, channel closed).
    WorkerDied,
}

impl Default for EarcutWorker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulate_simple_triangle() {
        let mut w = EarcutWorker::new();
        let verts = vec![[0.0, 0.0], [10.0, 0.0], [5.0, 10.0]];
        let indices = w
            .triangulate(verts, vec![], Duration::from_millis(100))
            .expect("simple triangle should triangulate");
        assert_eq!(indices.len(), 3);
    }

    #[test]
    fn timeout_returns_typed_error() {
        let mut w = EarcutWorker::new();
        // Zero-duration timeout will fire before the worker can respond
        // even on trivial input — easy way to exercise the timeout path.
        let verts = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let result = w.triangulate(verts, vec![], Duration::from_millis(0));
        assert!(matches!(result, Err(TriangulateError::TimedOut)));
    }

    #[test]
    fn triangulate_square_with_hole() {
        let mut w = EarcutWorker::new();
        let verts = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            // hole
            [3.0, 3.0],
            [7.0, 3.0],
            [7.0, 7.0],
            [3.0, 7.0],
        ];
        let holes = vec![4];
        let indices = w
            .triangulate(verts, holes, Duration::from_millis(100))
            .expect("square+hole should triangulate");
        assert!(!indices.is_empty());
        assert_eq!(indices.len() % 3, 0);
    }

    #[test]
    fn worker_survives_panic_and_serves_next_request() {
        let mut w = EarcutWorker::new();
        // Empty input — earcut may panic or produce no output. Either way
        // the worker should still serve subsequent requests.
        let _ = w.triangulate(vec![], vec![], Duration::from_millis(100));
        let verts = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let indices = w
            .triangulate(verts, vec![], Duration::from_millis(100))
            .expect("worker should serve request after empty-input call");
        assert_eq!(indices.len(), 3);
    }
}
