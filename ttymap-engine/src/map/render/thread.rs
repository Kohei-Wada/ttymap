//! Render thread — runs a RenderPipeline on a background thread.
//! Does not know about tiles, caching, or drawing internals.
//!
//! The loop is **purely event-driven** since #62: it parks on a
//! `crossbeam_channel::select!` over (task channel, decoder wake
//! channel) and never times out. Tile arrivals push-notify the
//! render thread directly, so the previous 50 ms upper-bound on
//! arrival-to-frame latency is gone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam_channel as cb;

use log::{debug, error, info};

use super::frame::MapFrame;
use super::pipeline::RenderPipeline;
use crate::map::Viewport;
use crate::map::styler::Styler;

/// Callback the render thread invokes for every completed frame.
/// Returns `false` to signal "consumer is gone, stop the thread".
/// The binary supplies a closure that wraps `MapFrame` into its own
/// bus protocol (e.g. `AppEvent::FrameReady`); the engine itself
/// stays oblivious to whatever bus the binary is using.
pub type FrameSink = Box<dyn FnMut(MapFrame) -> bool + Send>;

pub enum RenderTask {
    Draw {
        viewport: Viewport,
        overlays: Vec<crate::map::render::overlay::UserPolyline>,
    },
    Resize {
        width: usize,
        height: usize,
    },
    SetStyler(Arc<Styler>),
    /// Toggle tile text labels on / off. The render thread flips
    /// the renderer's flag in order with other tasks, so a
    /// `SetLabelsVisible` issued *between* two Draws will only
    /// affect the second.
    SetLabelsVisible(bool),
    Shutdown,
}

/// Cheap-to-clone client for issuing commands at the render thread.
/// Holds just the [`RenderTask`] sender — no thread or shutdown
/// state — so the App layer can hold a `RenderClient` without
/// owning the thread's lifecycle. The lifecycle owner is
/// [`RenderHandle`] (typically held by `main`).
#[derive(Clone)]
pub struct RenderClient {
    task_tx: cb::Sender<RenderTask>,
}

impl RenderClient {
    fn send(&self, task: RenderTask, label: &'static str) {
        if self.task_tx.send(task).is_err() {
            log::warn!("render thread channel closed on {}", label);
        }
    }

    pub fn request_draw(
        &self,
        viewport: Viewport,
        overlays: Vec<crate::map::render::overlay::UserPolyline>,
    ) {
        self.send(RenderTask::Draw { viewport, overlays }, "draw");
    }

    pub fn request_resize(&self, width: usize, height: usize) {
        self.send(RenderTask::Resize { width, height }, "resize");
    }

    /// Hand a fresh `Styler` to the render thread. Processed in order
    /// with `Draw` / `Resize`, so an in-flight frame at the old theme
    /// never collides with one at the new theme.
    pub fn set_styler(&self, styler: Arc<Styler>) {
        self.send(RenderTask::SetStyler(styler), "set_styler");
    }

    /// Toggle tile text labels on the render thread. Caller must
    /// follow up with a `request_draw` to see the change — this
    /// command alone only flips the flag.
    pub fn set_labels_visible(&self, visible: bool) {
        self.send(RenderTask::SetLabelsVisible(visible), "set_labels_visible");
    }
}

/// Lifecycle handle for the render thread: owns the `JoinHandle`
/// and the shutdown flag. The command sender lives on a peer
/// [`RenderClient`] which the App layer holds. `Drop` joins the
/// thread, so dropping the handle is the canonical shutdown.
pub struct RenderHandle {
    client: RenderClient,
    should_quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RenderHandle {
    /// Spawn the render thread.
    ///
    /// `wake_rx` is the push-notify channel from the decoder thread:
    /// each ping says "at least one tile arrived in the cache, drain
    /// it on the next render cycle". It replaces the polling timeout
    /// that used to bound tile-arrival → frame latency to 50 ms.
    ///
    /// `frame_sink` is a callback that receives every completed
    /// [`MapFrame`]. The binary wraps it into its own bus protocol
    /// (e.g. `AppEvent::FrameReady`); the engine itself stays
    /// oblivious. Returning `false` from the closure signals the
    /// consumer is gone and the thread should exit.
    pub fn spawn(
        pipeline: RenderPipeline,
        wake_rx: cb::Receiver<()>,
        frame_sink: FrameSink,
    ) -> Self {
        let (task_tx, task_rx) = cb::unbounded();
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = should_quit.clone();

        let thread = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_loop(task_rx, wake_rx, frame_sink, should_quit_clone, pipeline);
            }));
            if let Err(e) = result {
                let msg = if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                error!("RENDER THREAD PANICKED: {}", msg);
            }
        });

        RenderHandle {
            client: RenderClient { task_tx },
            should_quit,
            thread: Some(thread),
        }
    }

    /// Cheap-clone client for issuing render commands. Hand to the
    /// App so it can request redraws / resizes / theme changes
    /// without owning the thread.
    pub fn client(&self) -> RenderClient {
        self.client.clone()
    }

    pub fn shutdown(&mut self) {
        self.should_quit.store(true, Ordering::Relaxed);
        let _ = self.client.task_tx.send(RenderTask::Shutdown);
        // `Option::take` alone only **drops** the JoinHandle — that's
        // detach, not join. Wait for the thread to actually finish so
        // anything sequenced after shutdown sees it gone (issue #107).
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RenderHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Internal ──────────────────────────────────────────────────────────────────

/// What a single `RenderTask` leaves for the run loop to act on.
/// Draw is deferred to the caller because the two callers treat it
/// differently: the drain path keeps only the latest, the idle path
/// renders immediately.
enum TaskOutcome {
    Continue,
    Draw {
        viewport: Viewport,
        overlays: Vec<crate::map::render::overlay::UserPolyline>,
    },
    Shutdown,
}

fn apply_task(task: RenderTask, pipeline: &mut RenderPipeline) -> TaskOutcome {
    match task {
        RenderTask::Draw { viewport, overlays } => TaskOutcome::Draw { viewport, overlays },
        RenderTask::Resize { width, height } => {
            pipeline.resize(width, height);
            TaskOutcome::Continue
        }
        RenderTask::SetStyler(styler) => {
            pipeline.set_styler(styler);
            TaskOutcome::Continue
        }
        RenderTask::SetLabelsVisible(visible) => {
            pipeline.set_labels_visible(visible);
            TaskOutcome::Continue
        }
        RenderTask::Shutdown => TaskOutcome::Shutdown,
    }
}

/// Apply `first` followed by anything else already buffered on
/// `task_rx`. Returns the latest `Draw` viewport + overlays (older ones
/// are stale) or `Err(())` if a `Shutdown` was seen.
fn drain_tasks(
    first: RenderTask,
    task_rx: &cb::Receiver<RenderTask>,
    pipeline: &mut RenderPipeline,
) -> Result<Option<(Viewport, Vec<crate::map::render::overlay::UserPolyline>)>, ()> {
    let mut latest_draw: Option<(Viewport, Vec<crate::map::render::overlay::UserPolyline>)> = None;
    for task in std::iter::once(first).chain(task_rx.try_iter()) {
        match apply_task(task, pipeline) {
            TaskOutcome::Draw { viewport, overlays } => {
                latest_draw = Some((viewport, overlays));
            }
            TaskOutcome::Continue => {}
            TaskOutcome::Shutdown => return Err(()),
        }
    }
    Ok(latest_draw)
}

/// Hand a completed `MapFrame` to the [`FrameSink`] callback the
/// binary supplied. Returns `false` only when the consumer has gone
/// (closure returned `false`) — the run loop exits.
fn send_frame(sink: &mut FrameSink, frame: Option<MapFrame>) -> bool {
    if let Some(frame) = frame
        && !sink(frame)
    {
        return false;
    }
    true
}

fn run_loop(
    task_rx: cb::Receiver<RenderTask>,
    wake_rx: cb::Receiver<()>,
    mut frame_sink: FrameSink,
    should_quit: Arc<AtomicBool>,
    mut pipeline: RenderPipeline,
) {
    let mut last_viewport: Option<Viewport> = None;
    info!("render thread started");

    loop {
        if should_quit.load(Ordering::Relaxed) {
            break;
        }

        // Park until something happens. Either a task (Draw / Resize
        // / SetStyler / Shutdown) arrives on `task_rx`, or the
        // decoder pings `wake_rx` because at least one tile is now
        // in the cache.
        cb::select! {
            recv(task_rx) -> task => {
                // The select! arm pops one message; `drain_tasks`
                // walks `once(first).chain(rx.try_iter())` so the
                // first message and anything else already queued
                // collapse through the same path. Redundant draws
                // collapse to the latest viewport; side-effecting
                // tasks (Resize / SetStyler) apply in order.
                let first = match task {
                    Ok(t) => t,
                    Err(_) => break, // all senders dropped
                };
                let latest_draw = match drain_tasks(first, &task_rx, &mut pipeline) {
                    Err(()) => break,
                    Ok(d) => d,
                };
                match latest_draw {
                    Some((viewport, overlays)) => {
                        debug!("render: drawing (zoom={:.1}, overlays={})", viewport.zoom, overlays.len());
                        if !send_frame(&mut frame_sink, pipeline.render(&viewport, &overlays)) {
                            return;
                        }
                        last_viewport = Some(viewport);
                        // Prefetch on viewport changes only —
                        // anchored to user input, not idle ticks, so
                        // we don't trigger a feedback loop where
                        // prefetch arrivals wake the loop and queue
                        // more prefetch.
                        pipeline.prefetch(&viewport);
                    }
                    None => {
                        // Side-effecting task with no new viewport —
                        // re-render the previous one if we have one
                        // (e.g. theme change should refresh the
                        // visible frame).
                        if let Some(ref vp) = last_viewport
                            && !send_frame(&mut frame_sink, pipeline.render(vp, &[]))
                        {
                            return;
                        }
                    }
                }
            }
            recv(wake_rx) -> _ => {
                // Coalesce a burst of pings — the cache drain below
                // picks up everything regardless of count.
                while wake_rx.try_recv().is_ok() {}
                if let Some(ref viewport) = last_viewport
                    && pipeline.poll_tiles()
                    && !send_frame(&mut frame_sink, pipeline.render(viewport, &[]))
                {
                    return;
                }
            }
        }
    }

    info!("render thread exited");
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Regression for issue #107. `shutdown` previously did
    /// `self.thread.take()` which only **drops** the `JoinHandle` —
    /// `Drop for JoinHandle` is detach, not join. So control returned
    /// to the caller while the render thread was still running,
    /// contradicting CLAUDE.md's claim that "RenderHandle's thread
    /// shutdown is handled by its Drop impl".
    ///
    /// Probe: a thread that does a brief wind-down sleep before
    /// flipping a flag. After shutdown, the flag must be set —
    /// otherwise the join didn't actually wait.
    #[test]
    fn shutdown_joins_the_render_thread() {
        let exited = Arc::new(Mutex::new(false));
        let exited_clone = Arc::clone(&exited);

        let (task_tx, task_rx) = cb::unbounded::<RenderTask>();
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = Arc::clone(&should_quit);

        let thread = thread::spawn(move || {
            // Wait for the shutdown signal …
            loop {
                if should_quit_clone.load(Ordering::Relaxed) {
                    break;
                }
                match task_rx.recv_timeout(std::time::Duration::from_millis(20)) {
                    Ok(RenderTask::Shutdown) => break,
                    Ok(_) => {}
                    Err(cb::RecvTimeoutError::Disconnected) => break,
                    Err(cb::RecvTimeoutError::Timeout) => {}
                }
            }
            // … then do measurable wind-down work before exit.
            thread::sleep(std::time::Duration::from_millis(80));
            *exited_clone.lock().unwrap() = true;
        });

        let mut handle = RenderHandle {
            client: RenderClient { task_tx },
            should_quit,
            thread: Some(thread),
        };

        handle.shutdown();

        assert!(
            *exited.lock().unwrap(),
            "shutdown must wait for the render thread to finish (join), \
             not detach (drop the JoinHandle)"
        );
    }
}
