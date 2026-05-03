//! Opt-in file logging to the XDG state directory.
//!
//! Default: **no logger installed**. The TUI's logs are useful only
//! while debugging, and a default-on file logger that grows
//! unbounded over a multi-day session was adding cost (disk
//! pressure, runtime rotation complexity) for a payoff almost no
//! user ever collects. Most Rust TUI apps follow the same pattern
//! (yazi, gitui, bottom, nushell — all opt-in via env or flag).
//!
//! To enable, set `TTYMAP_LOG`:
//!
//! ```sh
//! TTYMAP_LOG=debug ttymap        # any level: error / warn / info / debug / trace
//! TTYMAP_LOG=1     ttymap        # alias for `debug`
//! ```
//!
//! When set, the file at `$XDG_STATE_HOME/ttymap/ttymap.log` is
//! **truncated on startup** (one debug session = one file). No
//! rotation: the user is consciously enabling logging for a
//! bounded run, and Ctrl-C + relaunch starts fresh. If a session
//! genuinely needs to span days, redirect with `tail -f` or pipe
//! to `logrotate(8)` like you would for any other long-running
//! program.
//!
//! When unset, [`init`] returns `Ok(None)` and `log::*!` macros
//! become no-ops (no logger registered).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;
use directories::ProjectDirs;
use log::{LevelFilter, Log, Metadata, Record};

const ENV_VAR: &str = "TTYMAP_LOG";

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

/// Parse the `TTYMAP_LOG` value into a `LevelFilter`. Accepts the
/// usual level names (`error` / `warn` / `info` / `debug` / `trace`)
/// case-insensitively, plus `1` as a shorthand for `debug`.
/// Anything unparseable returns `None` and the caller treats it as
/// "logging disabled" — easier to forgive a typo than to spam
/// stderr with parse errors when the user just typoed an env var.
fn level_from_env(value: &str) -> Option<LevelFilter> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "1" {
        return Some(LevelFilter::Debug);
    }
    trimmed.to_ascii_lowercase().parse::<LevelFilter>().ok()
}

/// Install a file logger when `TTYMAP_LOG` is set, otherwise
/// silently leave the global logger alone.
///
/// Returns:
/// - `Ok(Some(path))` when a logger was installed (debug session).
/// - `Ok(None)` when `TTYMAP_LOG` was unset / unparseable / `off`.
/// - `Err(_)` for filesystem failures while creating the log dir
///   or opening the file (caller can `.ok()` to ignore).
pub fn init() -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let raw = match std::env::var(ENV_VAR) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let level = match level_from_env(&raw) {
        Some(LevelFilter::Off) | None => return Ok(None),
        Some(level) => level,
    };

    let path = log_path().ok_or("could not determine log directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Truncate on startup: one TTYMAP_LOG run = one fresh file.
    // No runtime rotation needed when sessions are bounded by the
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
