//! IPC protocol for running the engine as a child subprocess.
//!
//! `ttymap` is one binary with two roles. The default role is the TUI;
//! when invoked as `ttymap engine-worker` (argv-dispatched at the very
//! top of `main` — before clap) the same binary becomes a headless
//! engine that talks to its parent over stdin/stdout.
//!
//! Wire format: each message is a u32 little-endian length followed by
//! a bincode-encoded payload. Parent → child carries [`EngineCommand`];
//! child → parent carries [`EngineEvent`]. Both directions are
//! independent — the parent need not block on a reply after sending a
//! command, and the child emits frames / state events whenever the
//! engine produces them.
//!
//! See `docs/architecture.md` and #348 for the multi-process design.

use std::io::{self, Read, Write};
use std::sync::mpsc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::geo::LonLat;
use crate::map::action::MapAction;
use crate::map::render::frame::MapFrame;
use crate::map::render::overlay::UserPolyline;
use crate::theme::ThemeId;

/// Parent → child commands. The first message after spawn must be
/// [`EngineCommand::Init`]; everything else is either rejected with an
/// [`EngineEvent::Error`] or interpreted as a shutdown intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineCommand {
    /// One-shot boot. Builds the tile cache + render pipeline + render
    /// thread inside the child. Reply: [`EngineEvent::Ready`].
    Init {
        config: Config,
        cols: u16,
        rows: u16,
        theme: ThemeId,
    },
    /// Terminal resize.
    Resize { cols: u16, rows: u16 },
    /// Swap the active theme (rebuilds the styler on the render thread).
    SetTheme(ThemeId),
    /// Toggle tile-rendered text labels. Caller is responsible for
    /// the follow-up [`EngineCommand::Redraw`].
    SetLabelsVisible(bool),
    /// Mutate engine state (pan / zoom / jump / reset / …).
    ApplyAction(MapAction),
    /// Trigger a fresh frame using the current viewport. `overlays` is
    /// the per-frame Lua-pushed polyline batch.
    Redraw { overlays: Vec<UserPolyline> },
    /// Cooperative shutdown. The child drops engine handles (which
    /// joins the render thread via `Drop`) and exits. EOF on stdin
    /// is treated the same way.
    Shutdown,
}

/// Child → parent events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineEvent {
    /// Handshake reply: engine has built and is ready for commands.
    /// Carries the tile backend's attribution string so the parent
    /// doesn't need an extra round-trip to read it (TileCache builds
    /// it during `crate::map::build`, IPC has no reason to lose it).
    Ready { attribution: Option<String> },
    /// A completed frame. ~430 KB at 240×80 (bincode-encoded).
    FrameReady(MapFrame),
    /// State mirror update. Emitted after any [`EngineCommand`] that
    /// may have changed the viewport (`Resize`, `ApplyAction`). The
    /// parent's UI-side mirror feeds Lua's synchronous getters
    /// (`ttymap.map:center()` etc.) without round-tripping IPC.
    ViewportChanged { center: LonLat, zoom: f64 },
    /// Protocol or runtime error from the child. Best-effort; the
    /// child may exit immediately after emitting this.
    Error(String),
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// Maximum payload size we accept on the wire. Sized to comfortably
/// hold a `MapFrame` from a very large terminal (~430 KB at 240×80)
/// plus headroom; rejecting anything larger guards against a buggy
/// or hostile peer flooding us with a multi-gigabyte length prefix.
const MAX_MESSAGE_BYTES: u32 = 16 * 1024 * 1024;

/// Write a length-prefixed bincode message. Caller is responsible for
/// flushing the writer when latency matters.
pub fn write_message<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let bytes = bincode::serde::encode_to_vec(msg, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "message exceeds u32 length"))?;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds MAX_MESSAGE_BYTES",
        ));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&bytes)?;
    Ok(())
}

/// Read one length-prefixed bincode message. Returns `Err` with
/// `UnexpectedEof` kind when the peer closes the pipe — callers
/// should treat that as a graceful shutdown signal.
pub fn read_message<T: DeserializeOwned, R: Read>(r: &mut R) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    r.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds MAX_MESSAGE_BYTES",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    let (msg, _consumed) = bincode::serde::decode_from_slice(&buf[..], bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(msg)
}

// ---------------------------------------------------------------------------
// Worker entry
// ---------------------------------------------------------------------------

/// Worker-role entry point. Reads `EngineCommand`s from stdin and
/// emits `EngineEvent`s on stdout until the parent closes the pipe
/// (EOF) or sends [`EngineCommand::Shutdown`]. Never returns.
///
/// All event writes funnel through one writer thread + an mpsc
/// channel so the render thread's [`FrameSink`](crate::map::render::thread::FrameSink)
/// and the command loop don't contend on stdout.
pub fn run_as_subprocess() -> ! {
    let stdin = io::stdin();
    let mut reader = io::BufReader::new(stdin.lock());

    let (event_tx, event_rx) = mpsc::channel::<EngineEvent>();

    let writer_handle = std::thread::Builder::new()
        .name("ttymap-engine-writer".into())
        .spawn(move || writer_loop(event_rx))
        .expect("spawn writer thread");

    let exit_code = command_loop(&mut reader, event_tx);

    // Dropping the last event_tx clone closes the channel; writer drains.
    let _ = writer_handle.join();

    std::process::exit(exit_code);
}

/// Drain `EngineEvent`s to stdout. Exits when the channel closes
/// (no more senders) or stdout breaks (parent dropped its end).
fn writer_loop(rx: mpsc::Receiver<EngineEvent>) {
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    while let Ok(ev) = rx.recv() {
        if write_message(&mut writer, &ev).is_err() {
            return;
        }
        // Flush every message: events are infrequent (~30/s during
        // active rendering) and IPC latency dominates over syscall
        // cost. Frame loss from a missed flush is harder to debug
        // than the modest write amplification.
        if writer.flush().is_err() {
            return;
        }
    }
    let _ = writer.flush();
}

/// Main command-dispatch loop. Returns the process exit code.
fn command_loop<R: Read>(reader: &mut R, event_tx: mpsc::Sender<EngineEvent>) -> i32 {
    // Init handshake. First message must be Init (or Shutdown for a
    // bare-bones spawn-and-exit smoke test).
    let cmd: EngineCommand = match read_message(reader) {
        Ok(c) => c,
        Err(_) => return 0, // EOF before any command — clean exit
    };
    let (config, cols, rows, theme) = match cmd {
        EngineCommand::Init {
            config,
            cols,
            rows,
            theme,
        } => (config, cols, rows, theme),
        EngineCommand::Shutdown => return 0,
        _ => {
            let _ = event_tx.send(EngineEvent::Error(
                "first command must be Init or Shutdown".into(),
            ));
            return 1;
        }
    };

    // Build the engine. FrameSink routes frames into the event channel.
    let frame_tx = event_tx.clone();
    let frame_sink: crate::map::render::thread::FrameSink =
        Box::new(move |frame| frame_tx.send(EngineEvent::FrameReady(frame)).is_ok());
    let (_render_handle, mut map) = match crate::map::build(&config, cols, rows, frame_sink, theme)
    {
        Ok(pair) => pair,
        Err(e) => {
            let _ = event_tx.send(EngineEvent::Error(format!("engine build failed: {e}")));
            return 1;
        }
    };

    if event_tx
        .send(EngineEvent::Ready {
            attribution: map.attribution.clone(),
        })
        .is_err()
    {
        return 1;
    }
    // Emit the initial viewport so the parent's mirror starts in sync.
    let _ = event_tx.send(EngineEvent::ViewportChanged {
        center: map.center(),
        zoom: map.zoom(),
    });

    while let Ok(cmd) = read_message::<EngineCommand, _>(reader) {
        match cmd {
            EngineCommand::Init { .. } => {
                // Re-init mid-session is out of scope; ignore.
            }
            EngineCommand::Resize { cols, rows } => {
                map.handle_resize(cols, rows);
                let _ = event_tx.send(EngineEvent::ViewportChanged {
                    center: map.center(),
                    zoom: map.zoom(),
                });
            }
            EngineCommand::SetTheme(theme) => {
                map.set_theme(theme);
            }
            EngineCommand::SetLabelsVisible(visible) => {
                map.set_labels_visible(visible);
            }
            EngineCommand::ApplyAction(action) => {
                let changed = map.apply_action(&action);
                if changed {
                    let _ = event_tx.send(EngineEvent::ViewportChanged {
                        center: map.center(),
                        zoom: map.zoom(),
                    });
                }
            }
            EngineCommand::Redraw { overlays } => {
                map.request_redraw(overlays);
            }
            EngineCommand::Shutdown => break,
        }
    }
    // Drop `map` first (releases RenderClient → render thread can exit
    // when its `_render_handle` is dropped at end of scope below).
    drop(map);
    drop(_render_handle);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode `value`, decode it back, hand the decoded value to
    /// `check`. EngineCommand / EngineEvent don't impl PartialEq
    /// transitively (Config / MapAction / MapFrame in chain), so the
    /// roundtrip assert is callsite-provided.
    fn roundtrip<T: Serialize + DeserializeOwned>(value: &T, check: impl FnOnce(T)) {
        let mut buf = Vec::new();
        write_message(&mut buf, value).expect("write");
        let mut cursor = io::Cursor::new(buf);
        let decoded: T = read_message(&mut cursor).expect("read");
        check(decoded);
    }

    #[test]
    fn command_init_round_trips() {
        let cmd = EngineCommand::Init {
            config: Config::default(),
            cols: 240,
            rows: 80,
            theme: ThemeId::Dark,
        };
        roundtrip(&cmd, |decoded| match decoded {
            EngineCommand::Init {
                cols,
                rows,
                theme,
                config,
            } => {
                assert_eq!(cols, 240);
                assert_eq!(rows, 80);
                assert_eq!(theme, ThemeId::Dark);
                assert_eq!(config.map.lat, Config::default().map.lat);
            }
            _ => panic!("expected Init"),
        });
    }

    #[test]
    fn command_resize_round_trips() {
        roundtrip(
            &EngineCommand::Resize {
                cols: 100,
                rows: 50,
            },
            |d| match d {
                EngineCommand::Resize { cols, rows } => {
                    assert_eq!(cols, 100);
                    assert_eq!(rows, 50);
                }
                _ => panic!("expected Resize"),
            },
        );
    }

    #[test]
    fn command_set_theme_round_trips() {
        roundtrip(&EngineCommand::SetTheme(ThemeId::Bright), |d| match d {
            EngineCommand::SetTheme(t) => assert_eq!(t, ThemeId::Bright),
            _ => panic!("expected SetTheme"),
        });
    }

    #[test]
    fn command_set_labels_visible_round_trips() {
        roundtrip(&EngineCommand::SetLabelsVisible(true), |d| match d {
            EngineCommand::SetLabelsVisible(v) => assert!(v),
            _ => panic!("expected SetLabelsVisible"),
        });
    }

    #[test]
    fn command_apply_action_jump_round_trips() {
        let cmd = EngineCommand::ApplyAction(MapAction::Jump(LonLat {
            lon: 139.76,
            lat: 35.68,
        }));
        roundtrip(&cmd, |d| match d {
            EngineCommand::ApplyAction(MapAction::Jump(ll)) => {
                assert_eq!(ll.lon, 139.76);
                assert_eq!(ll.lat, 35.68);
            }
            _ => panic!("expected ApplyAction(Jump)"),
        });
    }

    #[test]
    fn command_apply_action_fly_to_round_trips() {
        let cmd = EngineCommand::ApplyAction(MapAction::FlyTo {
            center: LonLat {
                lon: 13.42,
                lat: 52.51,
            },
            zoom: 10.5,
        });
        roundtrip(&cmd, |d| match d {
            EngineCommand::ApplyAction(MapAction::FlyTo { center, zoom }) => {
                assert_eq!(center.lon, 13.42);
                assert_eq!(zoom, 10.5);
            }
            _ => panic!("expected ApplyAction(FlyTo)"),
        });
    }

    #[test]
    fn command_redraw_round_trips() {
        let overlays = vec![UserPolyline {
            coords: vec![
                LonLat { lon: 0.0, lat: 0.0 },
                LonLat {
                    lon: 10.0,
                    lat: 10.0,
                },
            ],
            color: 12,
        }];
        roundtrip(&EngineCommand::Redraw { overlays }, |d| match d {
            EngineCommand::Redraw { overlays } => {
                assert_eq!(overlays.len(), 1);
                assert_eq!(overlays[0].coords.len(), 2);
                assert_eq!(overlays[0].color, 12);
            }
            _ => panic!("expected Redraw"),
        });
    }

    #[test]
    fn command_shutdown_round_trips() {
        roundtrip(&EngineCommand::Shutdown, |d| {
            assert!(matches!(d, EngineCommand::Shutdown))
        });
    }

    #[test]
    fn event_ready_round_trips() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &EngineEvent::Ready {
                attribution: Some("© OpenStreetMap".into()),
            },
        )
        .unwrap();
        let mut cursor = io::Cursor::new(buf);
        let decoded: EngineEvent = read_message(&mut cursor).unwrap();
        match decoded {
            EngineEvent::Ready { attribution } => {
                assert_eq!(attribution.as_deref(), Some("© OpenStreetMap"));
            }
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn event_viewport_changed_round_trips() {
        let mut buf = Vec::new();
        write_message(
            &mut buf,
            &EngineEvent::ViewportChanged {
                center: LonLat { lon: 1.0, lat: 2.0 },
                zoom: 7.5,
            },
        )
        .unwrap();
        let mut cursor = io::Cursor::new(buf);
        let decoded: EngineEvent = read_message(&mut cursor).unwrap();
        match decoded {
            EngineEvent::ViewportChanged { center, zoom } => {
                assert_eq!(center.lon, 1.0);
                assert_eq!(center.lat, 2.0);
                assert_eq!(zoom, 7.5);
            }
            other => panic!("expected ViewportChanged, got {other:?}"),
        }
    }

    #[test]
    fn event_frame_ready_round_trips() {
        use crate::map::render::frame::MapCell;
        let frame = MapFrame {
            cells: vec![MapCell {
                ch: 'a',
                fg: 1,
                bg: 0,
            }],
            cols: 1,
            rows: 1,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &EngineEvent::FrameReady(frame)).unwrap();
        let mut cursor = io::Cursor::new(buf);
        let decoded: EngineEvent = read_message(&mut cursor).unwrap();
        match decoded {
            EngineEvent::FrameReady(f) => {
                assert_eq!(f.cols, 1);
                assert_eq!(f.rows, 1);
                assert_eq!(f.cells.len(), 1);
                assert_eq!(f.cells[0].ch, 'a');
            }
            other => panic!("expected FrameReady, got {other:?}"),
        }
    }

    #[test]
    fn over_size_length_prefix_is_rejected() {
        let mut buf = Vec::new();
        let huge = (MAX_MESSAGE_BYTES + 1).to_le_bytes();
        buf.extend_from_slice(&huge);
        let mut cursor = io::Cursor::new(buf);
        let err = read_message::<EngineCommand, _>(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn truncated_message_surfaces_as_unexpected_eof() {
        // Length prefix says 100 bytes, payload is empty.
        let mut buf = Vec::new();
        buf.extend_from_slice(&100u32.to_le_bytes());
        let mut cursor = io::Cursor::new(buf);
        let err = read_message::<EngineCommand, _>(&mut cursor).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
