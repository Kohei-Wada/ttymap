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
use crate::core::RenderRequest;

pub enum RenderCommand {
    Draw(RenderRequest),
    Resize { width: usize, height: usize },
    Shutdown,
}

pub enum RenderResult {
    Frame(MapFrame),
}

pub struct RenderHandle {
    cmd_tx: mpsc::Sender<RenderCommand>,
    pub result_rx: mpsc::Receiver<RenderResult>,
    should_quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RenderHandle {
    pub fn spawn(pipeline: RenderPipeline) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let should_quit = Arc::new(AtomicBool::new(false));
        let should_quit_clone = should_quit.clone();

        let thread = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_loop(cmd_rx, result_tx, should_quit_clone, pipeline);
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
            cmd_tx,
            result_rx,
            should_quit,
            thread: Some(thread),
        }
    }

    pub fn request_draw(&self, state: RenderRequest) -> bool {
        self.cmd_tx.send(RenderCommand::Draw(state)).is_ok()
    }

    pub fn request_resize(&self, width: usize, height: usize) {
        if self
            .cmd_tx
            .send(RenderCommand::Resize { width, height })
            .is_err()
        {
            log::warn!("render thread channel closed on resize");
        }
    }

    pub fn shutdown(&mut self) {
        self.should_quit.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(RenderCommand::Shutdown);
        self.thread.take();
    }
}

impl Drop for RenderHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── Internal ──────────────────────────────────────────────────────────────────

fn drain_commands(
    cmd_rx: &mpsc::Receiver<RenderCommand>,
    pipeline: &mut RenderPipeline,
) -> Result<Option<RenderRequest>, ()> {
    let mut latest_draw: Option<RenderRequest> = None;

    loop {
        match cmd_rx.try_recv() {
            Ok(RenderCommand::Draw(state)) => latest_draw = Some(state),
            Ok(RenderCommand::Resize { width, height }) => pipeline.resize(width, height),
            Ok(RenderCommand::Shutdown) => return Err(()),
            Err(_) => break,
        }
    }
    Ok(latest_draw)
}

fn send_frame(result_tx: &mpsc::Sender<RenderResult>, frame: Option<MapFrame>) -> bool {
    if let Some(frame) = frame
        && result_tx.send(RenderResult::Frame(frame)).is_err()
    {
        return false; // channel closed
    }
    true
}

fn run_loop(
    cmd_rx: mpsc::Receiver<RenderCommand>,
    result_tx: mpsc::Sender<RenderResult>,
    should_quit: Arc<AtomicBool>,
    mut pipeline: RenderPipeline,
) {
    let mut last_state: Option<RenderRequest> = None;
    info!("render thread started");

    loop {
        if should_quit.load(Ordering::Relaxed) {
            break;
        }

        // 1. Drain commands — newest state wins
        match drain_commands(&cmd_rx, &mut pipeline) {
            Err(()) => break,
            Ok(Some(state)) => {
                debug!("render: drawing (zoom={:.1})", state.zoom);
                if !send_frame(&result_tx, pipeline.render(&state)) {
                    return;
                }
                last_state = Some(state);
                continue;
            }
            Ok(None) => {}
        }

        // 2. Poll tile completions
        if let Some(ref state) = last_state
            && pipeline.poll_tiles()
        {
            if !send_frame(&result_tx, pipeline.render(state)) {
                return;
            }
            continue;
        }

        // 3. Idle — prefetch
        if let Some(ref state) = last_state {
            pipeline.prefetch(state);
        }

        // 4. Wait for commands
        const POLL_MS: u64 = 50;
        match cmd_rx.recv_timeout(Duration::from_millis(POLL_MS)) {
            Ok(RenderCommand::Draw(state)) => {
                if !send_frame(&result_tx, pipeline.render(&state)) {
                    return;
                }
                last_state = Some(state);
            }
            Ok(RenderCommand::Resize { width, height }) => pipeline.resize(width, height),
            Ok(RenderCommand::Shutdown) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    info!("render thread exited");
}
