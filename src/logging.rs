//! Opt-in file logging to the XDG state directory.
//!
//! Default: **no logger installed**. The TUI's logs are useful only
//! while debugging, and a default-on file logger that grows
//! unbounded over a multi-day session was adding cost (disk
//! pressure, runtime rotation complexity) for a payoff almost no
//! user ever collects. Most Rust TUI apps follow the same pattern
//! (yazi, gitui, bottom, nushell — all opt-in via env or flag).
//!
//! Driven by the `--log [LEVEL]` CLI flag (parsed in `main.rs`):
//!
//! ```sh
//! ttymap --log              # implicit `debug`
//! ttymap --log debug        # explicit level
//! ttymap --log trace
//! ttymap                    # no logger; log::*! is a no-op
//! ```
//!
//! When set, the file at `$XDG_STATE_HOME/ttymap/ttymap.log` is
//! **truncated on startup** (one debug session = one file). No
//! rotation: the user is consciously enabling logging for a
//! bounded run, and Ctrl-C + relaunch starts fresh.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;
use directories::ProjectDirs;
use log::{LevelFilter, Log, Metadata, Record};

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let t = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(
                f,
                "[{} {} {}:{}] {}",
                t,
                record.level(),
                record.target(),
                record.line().unwrap_or(0),
                record.args()
            );
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

fn log_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    let state_dir = dirs
        .state_dir()
        .unwrap_or_else(|| dirs.data_local_dir())
        .to_path_buf();
    Some(state_dir.join("ttymap.log"))
}

/// Parse a level name (`error` / `warn` / `info` / `debug` /
/// `trace`) case-insensitively. Unknown strings fall back to
/// `Debug` rather than erroring — the user is asking for logs,
/// the worst case is they get more than they wanted.
fn parse_level(value: &str) -> LevelFilter {
    value
        .trim()
        .to_ascii_lowercase()
        .parse::<LevelFilter>()
        .unwrap_or(LevelFilter::Debug)
}

/// Install a file logger at the given level (or no-op when the
/// caller passes `"off"`). Truncates the file on startup so each
/// `--log` invocation starts fresh.
///
/// Returns:
/// - `Ok(Some(path))` when a logger was installed.
/// - `Ok(None)` when `level == "off"`.
/// - `Err(_)` for filesystem failures while creating the log dir
///   or opening the file.
pub fn init(level: &str) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let level = parse_level(level);
    if level == LevelFilter::Off {
        return Ok(None);
    }

    let path = log_path().ok_or("could not determine log directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Truncate on startup: one --log run = one fresh file. No
    // runtime rotation needed when sessions are bounded by the
    // user's choice to enable logging.
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;

    let logger = FileLogger {
        file: Mutex::new(file),
    };
    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(level);
    Ok(Some(path))
}
