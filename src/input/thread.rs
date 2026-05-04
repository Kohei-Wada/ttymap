//! Background thread that reads terminal events and forwards them as
//! [`AppEvent::Input`] onto the App's unified event queue.
//!
//! `crossterm::event::read()` blocks indefinitely; running it on the
//! main thread (the prior design) meant the loop had to use
//! `event::poll(timeout)` to stay responsive to render-thread frames,
//! Lua intents, and overlay redraw timing. Splitting input out lets
//! every source push into the same queue, so the main thread can
//! park on a single `recv_timeout` and react to whichever event
//! arrives first.
//!
//! Lifecycle mirrors [`crate::map::render::thread::RenderHandle`]:
//! a shared `should_quit` flag, polled inside the loop, plus a
//! `Drop` impl that signals the flag and joins. `crossterm` exposes
//! no primitive to interrupt a blocking `read()`, so the loop uses
//! `poll(timeout)` to check the flag every `poll_timeout` interval —
//! same idle wake cadence as the previous main-loop polling, just
//! relocated.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crossterm::event;
use log::warn;

use crate::app::AppEvent;

/// Owns the input-reader thread and its shutdown flag.
///
/// The thread sends [`AppEvent::Input`] for every terminal event it
/// reads. On `Drop` (or explicit [`Self::shutdown`]) it sets the flag
/// and joins; the thread sees the flag at the next poll-timeout
/// boundary and exits cleanly.
pub struct InputHandle {
    should_quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl InputHandle {
    /// Spawn the input thread.
    ///
    /// `event_tx` is a clone of the App-level [`AppEvent`] sender;
    /// each terminal event arrives wrapped as [`AppEvent::Input`].
    /// `poll_timeout` is the upper bound on shutdown latency — the
    /// loop calls [`event::poll`] with this interval, so a flagged
    /// quit is observed within one timeout.
    pub fn spawn(event_tx: mpsc::Sender<AppEvent>, poll_timeout: Duration) -> Self {
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = should_quit.clone();
        let thread = thread::spawn(move || run_loop(event_tx, poll_timeout, should_quit_clone));
        Self {
            should_quit,
            thread: Some(thread),
        }
    }

    /// Signal the thread to stop reading and join it. Idempotent:
    /// calling shutdown twice (or shutdown + Drop) is a no-op the
    /// second time because the JoinHandle is taken.
    pub fn shutdown(&mut self) {
        self.should_quit.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for InputHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_loop(
    event_tx: mpsc::Sender<AppEvent>,
    poll_timeout: Duration,
    should_quit: Arc<AtomicBool>,
) {
    while !should_quit.load(Ordering::Relaxed) {
        match event::poll(poll_timeout) {
            Ok(true) => match event::read() {
                Ok(ev) => {
                    if event_tx.send(AppEvent::Input(ev)).is_err() {
                        // Receiver dropped — App is tearing down.
                        return;
                    }
                }
                Err(e) => {
                    warn!("input thread: read failed: {}", e);
                }
            },
            Ok(false) => {
                // No event within the poll window; loop and re-check
                // the shutdown flag.
            }
            Err(e) => {
                warn!("input thread: poll failed: {}", e);
                // Brief sleep so a persistent error doesn't burn CPU.
                thread::sleep(poll_timeout);
            }
        }
    }
}
