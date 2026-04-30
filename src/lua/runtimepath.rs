//! Runtime path discovery — locates ordered list of ttymap data dirs
//! that hold bundled Lua plugins, lib scripts, and user overrides.
//!
//! ttymap follows the Neovim runtime-path model: multiple directories
//! contribute scripts in priority order. `require "ttymap.fmt"` walks
//! every layer until it finds a match; if `~/.config/ttymap/lua/ttymap/fmt.lua`
//! exists it wins over the bundled `~/.local/share/ttymap/lua/ttymap/fmt.lua`.
//!
//! Layers (highest priority first):
//!
//! 1. `$TTYMAP_RUNTIME` — env override, escape hatch for hackers /
//!    CI / multiple-checkouts.
//! 2. `$CARGO_MANIFEST_DIR/runtime` — `cargo run` from a git checkout
//!    finds the in-repo runtime/ automatically. Placed *before* XDG
//!    so a developer's live-edited source wins over any stale
//!    `make install` snapshot left in `~/.local/share/ttymap/`. On
//!    a user machine this path is the maintainer's home dir baked
//!    in at compile time and naturally doesn't exist, so it gets
//!    filtered out and the next layer wins.
//! 3. `$XDG_CONFIG_HOME/ttymap` (default `~/.config/ttymap`) —
//!    user-edited overrides. Drop a `lua/wiki.lua` here to shadow
//!    the bundled `wiki.lua`; this is also where `init.lua` lives
//!    for app config.
//! 4. `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`) —
//!    bundled scripts placed by `make install`.
//!
//! The Vec model is the foundation; `register_builtin_plugins` /
//! `install_builtin_searcher` walk it in order. PR1 wires the search
//! semantics; PR2 will add stem-dedup so a user-tier `wiki.lua`
//! registers as the only `wiki`.
//!
//! `cargo install` is intentionally not supported as a standalone
//! install path — the binary alone fails fast (with a "did you
//! `make install`?" message) when no layer resolves.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Set once at startup by [`crate::app`] after [`resolve_runtime_path`]
/// returns at least one valid layer. [`crate::lua::new_lua`] reads this
/// to wire the disk-based lib-script searcher and to extend
/// `package.path` with each layer's `lua/`.
static RUNTIME_PATH: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Errors returned by [`resolve_runtime_path`]. Carries the candidate
/// list so the caller can render a "we tried these paths" message.
pub struct RuntimePathError {
    pub candidates: Vec<PathBuf>,
}

impl std::fmt::Display for RuntimePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ttymap runtime not found. Tried (in order):")?;
        for p in &self.candidates {
            writeln!(f, "  - {}", p.display())?;
        }
        write!(
            f,
            "\nDid you `make install`? See README.md for install instructions."
        )
    }
}

/// Walk the layered resolution order and return every layer that
/// exists and has a `lua/` subdir. Order matches the module top
/// (env > user > bundled > dev). On full miss, returns the candidate
/// list back so the caller can render a "we tried these" failure.
pub fn resolve_runtime_path() -> Result<Vec<PathBuf>, RuntimePathError> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut tried: Vec<PathBuf> = Vec::new();

    let mut visit = |p: PathBuf| {
        if is_valid(&p) {
            found.push(p);
        } else {
            tried.push(p);
        }
    };

    if let Ok(p) = std::env::var("TTYMAP_RUNTIME") {
        visit(PathBuf::from(p));
    }
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        visit(PathBuf::from(manifest).join("runtime"));
    }
    if let Some(p) = xdg_config_runtime() {
        visit(p);
    }
    if let Some(p) = xdg_data_runtime() {
        visit(p);
    }

    if found.is_empty() {
        Err(RuntimePathError { candidates: tried })
    } else {
        Ok(found)
    }
}

/// Cache the resolved runtime path. Idempotent — first caller wins.
/// The app sets this once during startup; tests use
/// [`ensure_runtime_path_for_tests`].
pub fn set_runtime_path(path: Vec<PathBuf>) {
    let _ = RUNTIME_PATH.set(path);
}

/// Snapshot of the cached runtime path, in priority order. Empty
/// slice when the app hasn't resolved it yet — callers (`new_lua`
/// and its searcher) treat that as "no runtime layers reachable".
pub fn runtime_path() -> &'static [PathBuf] {
    RUNTIME_PATH.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Used by integration-style tests in this crate that exercise the
/// disk-based searcher and the runtime walker. Sets the runtime path
/// to the in-repo `runtime/` directory if no prior test has set it.
#[cfg(test)]
pub fn ensure_runtime_path_for_tests() {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let _ = RUNTIME_PATH.set(vec![dev]);
}

/// `true` iff `dir` exists and contains a `lua/` subdirectory — the
/// minimum shape every layer in the resolution order has to satisfy
/// for the lookup to consider it a runtime tier. Without the `lua/`
/// check, a bare directory would be accepted and the next
/// `register_builtin_plugins` call would silently load zero plugins.
fn is_valid(dir: &Path) -> bool {
    dir.is_dir() && dir.join("lua").is_dir()
}

/// `$XDG_CONFIG_HOME/ttymap` (default `~/.config/ttymap`). Holds
/// `init.lua` and any user override `lua/*.lua` scripts. `directories`
/// resolves the platform-specific equivalent on macOS / Windows.
fn xdg_config_runtime() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.config_dir().to_path_buf())
}

/// `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`). The
/// bundled scripts dir placed by `make install`.
fn xdg_data_runtime() -> Option<PathBuf> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("", "", "ttymap")?;
    Some(dirs.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_requires_lua_subdir() {
        let dir = std::env::temp_dir().join("ttymap-runtimepath-test-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_valid(&dir), "empty dir should not validate");

        std::fs::create_dir(dir.join("lua")).unwrap();
        assert!(is_valid(&dir), "dir with lua/ subdir validates");
    }

    #[test]
    fn is_valid_rejects_missing_dir() {
        let dir = std::env::temp_dir().join("ttymap-runtimepath-nope-xxx-yyy");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_valid(&dir));
    }

    #[test]
    fn manifest_dir_resolves_during_dev() {
        // The in-repo runtime/ directory always exists when running
        // tests — proves the dev fallback wires up.
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
        assert!(
            is_valid(&dev),
            "in-repo runtime/ must satisfy the validator"
        );
    }

    #[test]
    fn error_display_lists_candidates() {
        let err = RuntimePathError {
            candidates: vec![PathBuf::from("/a/b"), PathBuf::from("/c/d")],
        };
        let s = format!("{}", err);
        assert!(s.contains("/a/b"));
        assert!(s.contains("/c/d"));
        assert!(s.contains("make install"));
    }
}
