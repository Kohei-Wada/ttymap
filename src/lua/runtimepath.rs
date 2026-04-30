//! Runtime directory discovery — locates the on-disk ttymap data dir
//! that holds bundled Lua plugins.
//!
//! Bundled Lua plugin scripts and lib scripts are loaded from
//! `<runtime_dir>/lua/`. Until #183 they were `include_str!`'d into
//! the binary; the change to disk-based loading lets users `cp -r`
//! the directory and edit bundled scripts without recompiling, and
//! removes the silent two-sources-of-truth between `runtime/lua/*.lua`
//! and the deleted `BUILTIN_SCRIPTS` array.
//!
//! Resolution order (first hit wins):
//!
//! 1. `$TTYMAP_RUNTIME` — env override, optional escape hatch for
//!    hackers running multiple checkouts or CI smoke tests.
//! 2. `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`) —
//!    where `make install` places the runtime. The single canonical
//!    install path; we deliberately don't support system-wide
//!    `/etc/ttymap` or `/usr/local/share/ttymap` layouts because
//!    ttymap is single-user and root-installs aren't worth the path
//!    juggling.
//! 3. `$CARGO_MANIFEST_DIR/runtime` — `cargo run` from a git checkout
//!    finds the in-repo runtime/ automatically. Dev convenience.
//!
//! `cargo install` is intentionally not supported as a standalone
//! install path — the binary alone fails fast (with a "did you
//! `make install`?" message) when no runtime is found. See #183.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Set once at startup by [`crate::app`] after [`resolve_runtime_dir`]
/// succeeds. [`crate::lua::new_lua`] reads this to wire the disk-based
/// lib-script searcher and to extend `package.path` with
/// `<runtime>/lua/` so bundled plugins can `require` their siblings.
static RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Errors returned by [`resolve_runtime_dir`]. Carries the candidate
/// list so the caller can render a "we tried these paths" message.
pub struct RuntimeDirError {
    pub candidates: Vec<PathBuf>,
}

impl std::fmt::Display for RuntimeDirError {
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

/// Walk the resolution order documented at the module top and return
/// the first candidate that exists and contains a `lua/` subdirectory.
/// Returns the candidate list back to the caller on miss so the
/// failure message names every path tried.
pub fn resolve_runtime_dir() -> Result<PathBuf, RuntimeDirError> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. $TTYMAP_RUNTIME (env override)
    if let Ok(p) = std::env::var("TTYMAP_RUNTIME") {
        let p = PathBuf::from(p);
        if is_valid(&p) {
            return Ok(p);
        }
        candidates.push(p);
    }

    // 2. $XDG_DATA_HOME/ttymap (`make install` target)
    if let Some(p) = xdg_data_runtime() {
        if is_valid(&p) {
            return Ok(p);
        }
        candidates.push(p);
    }

    // 3. $CARGO_MANIFEST_DIR/runtime (dev path). `option_env!` rather
    //    than `env!` so a manually-rustc'd build (no cargo) still
    //    compiles.
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest).join("runtime");
        if is_valid(&p) {
            return Ok(p);
        }
        candidates.push(p);
    }

    Err(RuntimeDirError { candidates })
}

/// Cache `dir` so subsequent `crate::lua::new_lua` calls can wire the
/// disk-based searcher and extend `package.path`. Idempotent — first
/// caller wins, later attempts are silently ignored. The app sets
/// this once during startup; tests use [`ensure_runtime_dir_for_tests`].
pub fn set_runtime_dir(dir: PathBuf) {
    let _ = RUNTIME_DIR.set(dir);
}

/// Snapshot of the cached runtime dir. `None` when the app hasn't
/// resolved one yet — the caller (`new_lua` and its searcher) treats
/// that as "no bundled libs reachable" and falls through to the
/// standard `package.searchers`.
pub fn runtime_dir() -> Option<&'static Path> {
    RUNTIME_DIR.get().map(PathBuf::as_path)
}

/// Used by integration-style tests in this crate that exercise the
/// disk-based searcher (`builtin_searcher_resolves_ttymap_fmt`,
/// `every_bundled_script_registers`). Sets `RUNTIME_DIR` to the
/// in-repo `runtime/` directory if no prior test has set it.
#[cfg(test)]
pub fn ensure_runtime_dir_for_tests() {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let _ = RUNTIME_DIR.set(dev);
}

/// `true` iff `dir` exists and contains a `lua/` subdirectory — the
/// minimum shape every layer in the resolution order has to satisfy
/// for the lookup to consider it "the runtime dir". Without the
/// `lua/` check, a bare directory would short-circuit the search and
/// the next `register_builtin_plugins` call would silently load zero
/// plugins.
fn is_valid(dir: &Path) -> bool {
    dir.is_dir() && dir.join("lua").is_dir()
}

/// `$XDG_DATA_HOME/ttymap` (or platform equivalent — `directories`
/// resolves macOS / Windows correctly). On Linux this expands to
/// `~/.local/share/ttymap`. Returns `None` only when the host doesn't
/// expose a data dir at all.
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
        // Bare dir without lua/ — must not validate.
        assert!(!is_valid(&dir), "empty dir should not be a valid runtime");

        std::fs::create_dir(dir.join("lua")).unwrap();
        assert!(is_valid(&dir), "dir with lua/ subdir should validate");
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
        // tests — proves the dev fallback (#3) wires up.
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
        assert!(
            is_valid(&dev),
            "in-repo runtime/ must satisfy the validator"
        );
    }

    #[test]
    fn error_display_lists_candidates() {
        let err = RuntimeDirError {
            candidates: vec![PathBuf::from("/a/b"), PathBuf::from("/c/d")],
        };
        let s = format!("{}", err);
        assert!(s.contains("/a/b"));
        assert!(s.contains("/c/d"));
        assert!(s.contains("make install"));
    }
}
