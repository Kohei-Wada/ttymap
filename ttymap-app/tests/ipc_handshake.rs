//! End-to-end smoke test for the `ttymap engine-worker` IPC role.
//!
//! Spawns the actual binary as a child and drives it through the
//! Init → Ready → Draw → FrameReady → Shutdown round-trip, so both
//! directions of the codec see real payloads. Disk tile cache is disabled
//! (`CacheConfig.tiles = false`) so the test never touches `~/.cache/`
//! or the network — engine build resolves entirely in memory.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ttymap_engine::Config;
use ttymap_engine::geo::LonLat;
use ttymap_engine::ipc::{EngineCommand, EngineEvent, read_message, write_message};
use ttymap_engine::map::Viewport;
use ttymap_engine::theme::ThemeId;

/// Hard deadline for the whole handshake. Generous — engine build
/// is the slowest step and CI shared runners can be sluggish.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

fn workerless_config() -> Config {
    let mut config = Config::default();
    // Skip disk cache: no `~/.cache/` mkdir, no DiskCachedFetcher.
    // Engine still builds an in-memory tile cache + render thread.
    config.cache.tiles = false;
    config
}

#[test]
fn engine_worker_init_ready_shutdown_round_trip() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ttymap"))
        .arg("engine-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engine-worker");

    let mut stdin = child.stdin.take().expect("child stdin");
    let mut stdout = child.stdout.take().expect("child stdout");

    write_message(
        &mut stdin,
        &EngineCommand::Init {
            config: workerless_config(),
            // `config.cache.tiles == false`, so a `None` cache_dir
            // is semantically correct — engine never touches disk.
            cache_dir: None,
            width: 156,
            height: 84,
            theme: ThemeId::Dark,
        },
    )
    .expect("write Init");

    // Drain events until Ready.
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut saw_ready = false;
    while !saw_ready && Instant::now() < deadline {
        let ev: EngineEvent = read_message(&mut stdout).expect("read EngineEvent");
        if matches!(ev, EngineEvent::Ready { .. }) {
            saw_ready = true;
        }
    }
    assert!(saw_ready, "engine never emitted Ready within timeout");

    // One Draw, so a real `MapFrame` — the only large, deeply nested
    // payload in the protocol — makes the child → parent trip. With
    // tiles disabled the frame is blank, but its shape still has to
    // survive encode / decode.
    //
    // `Init` / `Viewport` dimensions are Braille *pixels*; the frame
    // comes back in terminal cells, 2 px wide by 4 px tall each.
    write_message(
        &mut stdin,
        &EngineCommand::Draw {
            viewport: Viewport {
                center: LonLat {
                    lon: 13.404954,
                    lat: 52.520008,
                },
                zoom: 4.0,
                width: 156,
                height: 84,
            },
            overlays: Vec::new(),
        },
    )
    .expect("write Draw");

    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut frame = None;
    while frame.is_none() && Instant::now() < deadline {
        match read_message(&mut stdout).expect("read EngineEvent") {
            EngineEvent::FrameReady(f) => frame = Some(f),
            EngineEvent::Error(e) => panic!("engine reported error: {e}"),
            EngineEvent::Ready { .. } => {}
        }
    }
    let frame = frame.expect("engine never emitted FrameReady within timeout");
    assert_eq!(frame.cols, 156 / 2);
    assert_eq!(frame.rows, 84 / 4);
    assert_eq!(frame.cells.len(), (156 / 2) * (84 / 4));
    assert!((frame.zoom - 4.0).abs() < 1e-9);
    assert!((frame.center.lon - 13.404954).abs() < 1e-9);

    // Cooperative shutdown.
    write_message(&mut stdin, &EngineCommand::Shutdown).expect("write Shutdown");
    drop(stdin);

    // Drain any remaining stdout to EOF so the child's writer side
    // never blocks. Run in a thread so the main test can wait on
    // exit status with a deadline.
    let drain = thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = stdout.read_to_end(&mut sink);
    });

    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("engine-worker did not exit within {HANDSHAKE_TIMEOUT:?}");
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    let _ = drain.join();

    assert!(status.success(), "engine-worker exited with {status:?}");
}

/// EOF on stdin before any command is a clean exit (parent gone /
/// supervised teardown). The child must not panic or write to stderr.
#[test]
fn engine_worker_exits_cleanly_on_stdin_eof_before_init() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ttymap"))
        .arg("engine-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn engine-worker");

    // Drop stdin immediately — child sees EOF before any message.
    drop(child.stdin.take());

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("engine-worker did not exit on stdin EOF");
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    };
    assert!(status.success(), "engine-worker exited with {status:?}");
}
