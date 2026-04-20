//! `ttymap clear-cache` — remove the on-disk tile cache.

use std::fs;
use std::io;

pub fn run() -> io::Result<()> {
    let cache_dir = directories::ProjectDirs::from("", "", "ttymap")
        .map(|dirs| dirs.cache_dir().to_path_buf());

    match cache_dir {
        Some(dir) if dir.exists() => match fs::remove_dir_all(&dir) {
            Ok(()) => println!("Cleared tile cache: {}", dir.display()),
            Err(e) => eprintln!("Failed to clear cache: {e}"),
        },
        Some(dir) => println!("No cache to clear: {}", dir.display()),
        None => eprintln!("Could not determine cache directory"),
    }
    Ok(())
}
