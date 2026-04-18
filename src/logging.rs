//! File-based logging to XDG state directory with automatic rotation.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;
use directories::ProjectDirs;
use log::{LevelFilter, Log, Metadata, Record};

/// Maximum log file size before rotation (1 MB).
const MAX_LOG_SIZE: u64 = 1_024 * 1_024;

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let t = Local::now().format("%H:%M:%S%.3f");
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

/// Rotate the log file if it exceeds MAX_LOG_SIZE.
/// Renames current log to ttymap.log.old, then starts fresh.
fn rotate_if_needed(path: &PathBuf) {
    if let Ok(meta) = fs::metadata(path)
        && meta.len() > MAX_LOG_SIZE
    {
        let old = path.with_extension("log.old");
        let _ = fs::rename(path, old);
    }
}

/// Initialize file-based logging to `$XDG_STATE_HOME/ttymap/ttymap.log`.
/// Rotates the log file if it exceeds 1 MB.
pub fn init() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = log_path().ok_or("could not determine log directory")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    rotate_if_needed(&path);

    let file = OpenOptions::new().create(true).append(true).open(&path)?;

    let logger = FileLogger {
        file: Mutex::new(file),
    };
    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Debug);
    Ok(path)
}
