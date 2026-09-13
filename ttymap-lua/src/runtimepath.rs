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
//! 2. `$CARGO_MANIFEST_DIR/../runtime` — `cargo run` from a git
//!    checkout finds the in-repo `runtime/` at the workspace root
//!    automatically (this crate lives under `ttymap-lua/`, the
//!    runtime sits one level up). Placed *before* XDG so a
//!    developer's live-edited source wins over any stale
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
//! 5. `/usr/local/share/ttymap`, then `/usr/share/ttymap` — the
//!    system tiers a distro package (AUR, …) writes into. Last in
//!    priority so a per-user `make install` always shadows a
//!    packaged copy, and absent on a machine that has never
//!    installed one.
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

use ttymap_config::AppDirs;

/// Set once at startup by [`crate::app`] after [`resolve_runtime_path`]
/// returns at least one valid layer. [`crate::new_lua`] reads this
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
pub fn resolve_runtime_path(dirs: Option<&AppDirs>) -> Result<Vec<PathBuf>, RuntimePathError> {
    let (found, tried): (Vec<PathBuf>, Vec<PathBuf>) = candidate_layers(dirs)
        .into_iter()
        .partition(|p| is_valid(p));

    if found.is_empty() {
        Err(RuntimePathError { candidates: tried })
    } else {
        Ok(found)
    }
}

/// System-wide tiers, highest priority first. A distro package
/// (AUR, …) installs under one of these; `make install` never
/// writes here, so on a machine without a packaged ttymap they
/// simply don't exist and drop out of the resolved list.
const SYSTEM_LAYERS: [&str; 2] = ["/usr/local/share/ttymap", "/usr/share/ttymap"];

/// Every layer we would consider, in priority order, before checking
/// whether any of them exists. Split out from [`resolve_runtime_path`]
/// so the ordering itself is testable without staging directories.
fn candidate_layers(dirs: Option<&AppDirs>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if let Ok(p) = std::env::var("TTYMAP_RUNTIME") {
        out.push(PathBuf::from(p));
    }
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        // ttymap-lua/ lives under the workspace root; runtime/ sits
        // at that root, one level up from this crate's manifest.
        if let Some(workspace) = PathBuf::from(manifest).parent() {
            out.push(workspace.join("runtime"));
        }
    }
    // The user tier (`$XDG_CONFIG_HOME/ttymap`) holds the user's
    // overrides; the bundled tier (`$XDG_DATA_HOME/ttymap`) is what
    // `make install` populates. Both come from the centralised
    // `AppDirs` resolver in `ttymap-config` (#362).
    if let Some(d) = dirs {
        out.push(d.config.clone());
        out.push(d.data.clone());
    }
    out.extend(SYSTEM_LAYERS.iter().map(PathBuf::from));

    out
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
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ttymap-lua crate must have a parent (workspace root)")
        .join("runtime");
    let _ = RUNTIME_PATH.set(vec![dev]);
}

/// `true` iff `dir` exists. The layer's internal layout
/// (`lua/`, `lua/plugin/`, etc.) is not Rust's concern — bundled
/// `runtime/init.lua` and user `init.lua` decide what to
/// `require` under each layer. The manifest path baked at compile
/// time naturally filters out on user machines because it
/// doesn't exist there.
fn is_valid(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_accepts_any_existing_directory() {
        // The layer's internal layout is Lua-side concern (the
        // plugin searcher decides what `<layer>/<sub>` to look at).
        // Rust just confirms the path exists and is a directory.
        let dir = std::env::temp_dir().join("ttymap-runtimepath-test-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(is_valid(&dir), "any existing dir validates");
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
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("ttymap-lua crate must have a parent (workspace root)")
            .join("runtime");
        assert!(
            is_valid(&dev),
            "in-repo runtime/ must satisfy the validator"
        );
    }

    #[test]
    fn system_layers_rank_below_the_per_user_tiers() {
        // Order is the whole contract: a per-user `make install`
        // under $XDG_DATA_HOME must shadow a distro-packaged copy
        // under /usr/share, never the other way round.
        let dirs = AppDirs {
            config: PathBuf::from("/tmp/ttymap-test-config"),
            data: PathBuf::from("/tmp/ttymap-test-data"),
            cache: PathBuf::from("/tmp/ttymap-test-cache"),
            state: PathBuf::from("/tmp/ttymap-test-state"),
        };
        let layers = candidate_layers(Some(&dirs));

        let pos = |p: &str| {
            layers
                .iter()
                .position(|l| l == &PathBuf::from(p))
                .unwrap_or_else(|| panic!("{p} missing from candidates: {layers:?}"))
        };

        assert!(pos("/tmp/ttymap-test-config") < pos("/tmp/ttymap-test-data"));
        assert!(pos("/tmp/ttymap-test-data") < pos("/usr/local/share/ttymap"));
        assert!(pos("/usr/local/share/ttymap") < pos("/usr/share/ttymap"));
    }

    #[test]
    fn system_layers_are_offered_even_without_xdg_dirs() {
        // `AppDirs::resolve()` can come back None (no HOME). A
        // packaged install must still be reachable in that case.
        let layers = candidate_layers(None);
        assert!(layers.contains(&PathBuf::from("/usr/share/ttymap")));
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
