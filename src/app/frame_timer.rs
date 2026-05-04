//! Background thread that emits [`AppEvent::Wake`] on the unified
//! event queue at a fixed cadence.
//!
//! Replaces the old `event_rx.recv_timeout(poll_timeout)` pattern in
//! the main loop: the loop now blocks on `recv()`, and this timer
//! keeps it unblocked at the same rate the timeout used to drive.
//! Per-frame work (animation `on_tick` callbacks, overlay redraw
//! rate-check) therefore still ticks predictably even with no input,
//! render, or Lua intent activity.
//!
//! Lifecycle mirrors [`ttymap_engine::map::render::thread::RenderHandle`] and
//! [`super::input_thread::InputHandle`]: a shared `should_quit` flag
//! is checked between each sleep + send; on `Drop` (or explicit
//! [`Self::shutdown`]) the flag flips and the thread exits within
//! one tick interval.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::AppEvent;

/// Owns the frame-timer thread and its shutdown flag.
pub struct FrameTimer {
    should_quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FrameTimer {
    /// Spawn the timer.
    ///
    /// `event_tx` is a clone of the App-level [`AppEvent`] sender;
    /// each tick arrives wrapped as [`AppEvent::Wake`]. `interval`
    /// is the cadence — typically the same `poll_timeout_ms` value
    /// the prior `recv_timeout` block used (default 50 ms = 20 Hz),
    /// keeping animation behaviour and idle CPU equivalent.
    pub fn spawn(event_tx: mpsc::Sender<AppEvent>, interval: Duration) -> Self {
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = should_quit.clone();
        let thread = thread::spawn(move || run_loop(event_tx, interval, should_quit_clone));
        Self {
            should_quit,
            thread: Some(thread),
        }
    }

    /// Signal the thread to stop and join it. Idempotent: a second
    /// call (e.g. via Drop after explicit shutdown) is a no-op.
    pub fn shutdown(&mut self) {
        self.should_quit.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_loop(event_tx: mpsc::Sender<AppEvent>, interval: Duration, should_quit: Arc<AtomicBool>) {
    while !should_quit.load(Ordering::Relaxed) {
        thread::sleep(interval);
        if should_quit.load(Ordering::Relaxed) {
            break;
        }
        if event_tx.send(AppEvent::Wake).is_err() {
            // App has dropped the receiver — teardown.
            return;
        }
    }
}
