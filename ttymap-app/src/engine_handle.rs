//! `EngineHandle` — TUI-side IPC transport to the `ttymap
//! engine-worker` subprocess.
//!
//! Pure transport: spawns the same binary as a child via
//! `Command::new(current_exe).arg("engine-worker")`, pipes a bincode-
//! framed [`EngineCommand`] / [`EngineEvent`] stream over
//! stdin/stdout, and exposes thin `send_*` methods that the App
//! calls after mutating its own state.
//!
//! The UI-side mirror of [`MapState`] lives on `App` (see
//! `ttymap-app/src/app/mod.rs`'s `map_state` field) — the App
//! mutates it synchronously in `dispatch`, then forwards the same
//! `MapAction` to the child here. Both sides run the identical
//! transitions on the same inputs, so they stay coherent by
//! construction without round-trip RPCs.
//!
//! `snap` keeps using `ttymap_engine::map::build` in-process; only
//! the long-lived TUI takes the subprocess path. See #348 for the
//! full multi-process design.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ttymap_engine::Config as EngineConfig;
use ttymap_engine::ipc::{EngineCommand, EngineEvent, read_message, write_message};
use ttymap_engine::map::Viewport;
use ttymap_engine::map::render::overlay::UserPolyline;
use ttymap_engine::theme::ThemeId;

use crate::app::AppEvent;

/// Errors surfaced from [`EngineHandle::spawn`]. Anything that
/// happens after spawn (writer / reader thread failures, child
/// exit) is logged and surfaces as a dead-engine state — Phase 3
/// will add restart logic.
#[derive(Debug)]
pub enum EngineHandleError {
    /// Couldn't resolve the parent binary path to spawn the child.
    CurrentExe(io::Error),
    /// Failed to spawn the child or set up its pipes.
    Spawn(io::Error),
    /// Wire I/O failed during the Init handshake.
    Handshake(io::Error),
    /// Child emitted an `EngineEvent::Error` before Ready.
    Subprocess(String),
    /// Child closed stdout before sending Ready.
    UnexpectedEof,
}

impl std::fmt::Display for EngineHandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentExe(e) => write!(f, "resolve current exe: {e}"),
            Self::Spawn(e) => write!(f, "spawn engine-worker: {e}"),
            Self::Handshake(e) => write!(f, "engine-worker handshake I/O: {e}"),
            Self::Subprocess(msg) => write!(f, "engine-worker reported error: {msg}"),
            Self::UnexpectedEof => write!(f, "engine-worker exited before Ready"),
        }
    }
}

impl std::error::Error for EngineHandleError {}

pub struct EngineHandle {
    /// Command stream. Cleared in [`Drop`] to signal the writer
    /// thread to exit (which closes child stdin → child exits).
    command_tx: Option<mpsc::Sender<EngineCommand>>,
    /// Tile-backend attribution string, captured from the Ready event.
    pub attribution: Option<String>,
    child: Option<Child>,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Spawn the engine-worker child, run the Init handshake, and
    /// return a handle wired to App's event channel. Blocks until
    /// the child emits Ready or exits with an error.
    pub fn spawn(
        config: &EngineConfig,
        cache_dir: Option<std::path::PathBuf>,
        cols: u16,
        rows: u16,
        theme: ThemeId,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Result<Self, EngineHandleError> {
        let exe = std::env::current_exe().map_err(EngineHandleError::CurrentExe)?;
        let mut child = Command::new(exe)
            .arg("engine-worker")
            // child stderr → parent stderr so engine-side panics or
            // log messages are visible in the user's terminal session.
            .stderr(Stdio::inherit())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(EngineHandleError::Spawn)?;

        let mut child_stdin = BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| EngineHandleError::Spawn(io::Error::other("missing stdin")))?,
        );
        let mut child_stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| EngineHandleError::Spawn(io::Error::other("missing stdout")))?,
        );

        // Init handshake. Send Init synchronously, then drain events
        // until Ready arrives. Anything before Ready is unexpected
        // (the worker only emits Ready / Error / FrameReady — and
        // FrameReady can't fire before render thread starts, which
        // is after Ready).
        write_message(
            &mut child_stdin,
            &EngineCommand::Init {
                config: config.clone(),
                cache_dir,
                cols,
                rows,
                theme,
            },
        )
        .and_then(|()| child_stdin.flush())
        .map_err(EngineHandleError::Handshake)?;

        let attribution = loop {
            let ev: EngineEvent = read_message(&mut child_stdout).map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => EngineHandleError::UnexpectedEof,
                _ => EngineHandleError::Handshake(e),
            })?;
            match ev {
                EngineEvent::Ready { attribution } => break attribution,
                EngineEvent::Error(msg) => return Err(EngineHandleError::Subprocess(msg)),
                _ => {
                    // Tolerate spurious events; loop back.
                }
            }
        };

        let (command_tx, command_rx) = mpsc::channel::<EngineCommand>();
        let writer = thread::Builder::new()
            .name("ttymap-engine-writer".into())
            .spawn(move || writer_loop(child_stdin, command_rx))
            .map_err(|e| EngineHandleError::Spawn(io::Error::other(e.to_string())))?;

        let reader = thread::Builder::new()
            .name("ttymap-engine-reader".into())
            .spawn(move || reader_loop(child_stdout, event_tx))
            .map_err(|e| EngineHandleError::Spawn(io::Error::other(e.to_string())))?;

        Ok(Self {
            command_tx: Some(command_tx),
            attribution,
            child: Some(child),
            writer: Some(writer),
            reader: Some(reader),
        })
    }

    // ── Wire-format senders ─────────────────────────────────────────────
    //
    // Each `send_*` is a thin shim around the underlying `send` —
    // the App mutates its own `MapState` mirror first, then calls
    // these to forward the same intent to the child. The two sides
    // run identical transitions on identical inputs and stay
    // coherent by construction.

    pub fn send_resize(&self, cols: u16, rows: u16) {
        self.send(EngineCommand::Resize { cols, rows });
    }

    pub fn set_theme(&self, theme: ThemeId) {
        self.send(EngineCommand::SetTheme(theme));
    }

    pub fn set_labels_visible(&self, visible: bool) {
        self.send(EngineCommand::SetLabelsVisible(visible));
    }

    pub fn set_layer_visible(&self, layer: &str, visible: bool) {
        self.send(EngineCommand::SetLayerVisible {
            layer: layer.to_string(),
            visible,
        });
    }

    pub fn request_redraw(&self, viewport: Viewport, overlays: Vec<UserPolyline>) {
        self.send(EngineCommand::Draw { viewport, overlays });
    }

    fn send(&self, cmd: EngineCommand) {
        if let Some(tx) = &self.command_tx
            && tx.send(cmd).is_err()
        {
            // Writer thread is gone — engine has died. Phase 3 will
            // notice this and respawn; today, just log and let the
            // App keep running without redraws (frames stop arriving
            // but the UI doesn't crash).
            log::warn!("engine-worker writer is gone; command dropped");
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Cooperative teardown: send Shutdown, drop the command tx so
        // the writer thread exits (which closes child stdin → child
        // breaks its read loop → exits → stdout EOF → reader thread
        // returns). Then join everything and reap the child.
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(EngineCommand::Shutdown);
            drop(tx);
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        if let Some(mut child) = self.child.take() {
            // Bounded wait — if the child hangs we shouldn't block
            // shutdown forever. 1 s is generous: cooperative exit
            // should take milliseconds.
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if std::time::Instant::now() >= deadline => {
                        log::warn!("engine-worker did not exit within 1s; killing");
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        }
    }
}

fn writer_loop(mut stdin: BufWriter<impl Write>, rx: mpsc::Receiver<EngineCommand>) {
    while let Ok(cmd) = rx.recv() {
        if write_message(&mut stdin, &cmd).is_err() {
            return;
        }
        if stdin.flush().is_err() {
            return;
        }
    }
    let _ = stdin.flush();
}

fn reader_loop(mut stdout: BufReader<impl Read>, event_tx: mpsc::Sender<AppEvent>) {
    while let Ok(ev) = read_message::<EngineEvent, _>(&mut stdout) {
        match ev {
            EngineEvent::FrameReady(frame) => {
                if event_tx.send(AppEvent::FrameReady(frame)).is_err() {
                    return; // App is gone
                }
            }
            EngineEvent::Ready { .. } => {
                // Spurious — already consumed during handshake.
            }
            EngineEvent::Error(msg) => {
                log::error!("engine-worker error: {msg}");
            }
        }
    }
}
