//! Render thread — runs a RenderPipeline on a background thread.
//! Does not know about tiles, caching, or drawing internals.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use log::{debug, error, info};

use super::frame::MapFrame;
use super::pipeline::RenderPipeline;
use crate::map::Viewport;
use crate::map::styler::Styler;

pub enum RenderTask {
    Draw(Viewport),
    Resize { width: usize, height: usize },
    SetStyler(Arc<Styler>),
    Shutdown,
}

pub struct RenderHandle {
    task_tx: mpsc::Sender<RenderTask>,
    frame_rx: mpsc::Receiver<MapFrame>,
    should_quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RenderHandle {
    pub fn spawn(pipeline: RenderPipeline) -> Self {
        let (task_tx, task_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = should_quit.clone();

        let thread = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_loop(task_rx, frame_tx, should_quit_clone, pipeline);
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
            task_tx,
            frame_rx,
            should_quit,
            thread: Some(thread),
        }
    }

    pub fn request_draw(&self, viewport: Viewport) {
        if self.task_tx.send(RenderTask::Draw(viewport)).is_err() {
            log::warn!("render thread channel closed on draw");
        }
    }

    /// Pull the next completed frame from the render thread, if any.
    /// Non-blocking: returns `None` when the queue is empty.
    pub fn try_recv_frame(&self) -> Option<MapFrame> {
        self.frame_rx.try_recv().ok()
    }

    pub fn request_resize(&self, width: usize, height: usize) {
        if self
            .task_tx
            .send(RenderTask::Resize { width, height })
            .is_err()
        {
            log::warn!("render thread channel closed on resize");
        }
    }

    /// Hand a fresh `Styler` to the render thread. Processed in order
    /// with `Draw` / `Resize`, so an in-flight frame at the old theme
    /// never collides with one at the new theme.
    pub fn set_styler(&self, styler: Arc<Styler>) {
        if self.task_tx.send(RenderTask::SetStyler(styler)).is_err() {
            log::warn!("render thread channel closed on set_styler");
        }
    }

    pub fn shutdown(&mut self) {
        self.should_quit.store(true, Ordering::Relaxed);
        let _ = self.task_tx.send(RenderTask::Shutdown);
        self.thread.take();
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
    Draw(Viewport),
    Shutdown,
}

fn apply_task(task: RenderTask, pipeline: &mut RenderPipeline) -> TaskOutcome {
    match task {
        RenderTask::Draw(viewport) => TaskOutcome::Draw(viewport),
        RenderTask::Resize { width, height } => {
            pipeline.resize(width, height);
            TaskOutcome::Continue
        }
        RenderTask::SetStyler(styler) => {
            pipeline.set_styler(styler);
            TaskOutcome::Continue
        }
        RenderTask::Shutdown => TaskOutcome::Shutdown,
    }
}

fn drain_tasks(
    task_rx: &mpsc::Receiver<RenderTask>,
    pipeline: &mut RenderPipeline,
) -> Result<Option<Viewport>, ()> {
    let mut latest_draw: Option<Viewport> = None;

    while let Ok(task) = task_rx.try_recv() {
        match apply_task(task, pipeline) {
            TaskOutcome::Draw(viewport) => latest_draw = Some(viewport),
            TaskOutcome::Continue => {}
            TaskOutcome::Shutdown => return Err(()),
        }
    }
    Ok(latest_draw)
}

fn send_frame(frame_tx: &mpsc::Sender<MapFrame>, frame: Option<MapFrame>) -> bool {
    if let Some(frame) = frame
        && frame_tx.send(frame).is_err()
    {
        return false; // channel closed
    }
    true
}

fn run_loop(
    task_rx: mpsc::Receiver<RenderTask>,
    frame_tx: mpsc::Sender<MapFrame>,
    should_quit: Arc<AtomicBool>,
    mut pipeline: RenderPipeline,
) {
    let mut last_viewport: Option<Viewport> = None;
    info!("render thread started");

    loop {
        if should_quit.load(Ordering::Relaxed) {
            break;
        }

        // 1. Drain tasks — newest viewport wins
        match drain_tasks(&task_rx, &mut pipeline) {
            Err(()) => break,
            Ok(Some(viewport)) => {
                debug!("render: drawing (zoom={:.1})", viewport.zoom);
                if !send_frame(&frame_tx, pipeline.render(&viewport)) {
                    return;
                }
                last_viewport = Some(viewport);
                continue;
            }
            Ok(None) => {}
        }

        // 2. Poll tile completions
        if let Some(ref viewport) = last_viewport
            && pipeline.poll_tiles()
        {
            if !send_frame(&frame_tx, pipeline.render(viewport)) {
                return;
            }
            continue;
        }

        // 3. Idle — prefetch
        if let Some(ref viewport) = last_viewport {
            pipeline.prefetch(viewport);
        }

        // 4. Wait for tasks
        const POLL_MS: u64 = 50;
        match task_rx.recv_timeout(Duration::from_millis(POLL_MS)) {
            Ok(task) => match apply_task(task, &mut pipeline) {
                TaskOutcome::Draw(viewport) => {
                    if !send_frame(&frame_tx, pipeline.render(&viewport)) {
                        return;
                    }
                    last_viewport = Some(viewport);
                }
                TaskOutcome::Continue => {}
                TaskOutcome::Shutdown => break,
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    info!("render thread exited");
}
