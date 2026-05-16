//! Resolved XDG directory paths for ttymap.
//!
//! Pre-#362, every crate (engine, app, lua, …) called
//! `directories::ProjectDirs::from("", "", "ttymap")` independently
//! — the brand string `"ttymap"` was hardcoded at 7 sites. This
//! module centralises the resolution so the brand and the XDG
//! fallback policy (state → data_local_dir) live in one place.
//!
//! `AppDirs::resolve()` is called once at process startup
//! (`ttymap-app/src/main.rs` and `ttymap-cli/src/snap.rs`); the
//! resulting struct is threaded into every consumer:
//!
//! - **engine tile cache** — `cache` flows via
//!   `EngineCommand::Init.cache_dir` over the IPC pipe.
//! - **app log file** — `state.join("ttymap.log")`.
//! - **lua runtime path** — `config` and `data` feed the layered
//!   resolver.
//! - **lua HTTP cache** — `cache.join("lua-http/")`.
//! - **lua storage lib** — `data.join("storage/")`.
//! - **bundled init.lua** — `config` is used to skip the user tier
//!   when walking the runtime path for the bundled file.
//!
//! `resolve()` returns `Option<Self>` to mirror the pre-existing
//! "no $HOME / no per-user dirs" failure mode that every original
//! call site already handled. Consumers gracefully degrade (no
//! cache, no log) when `None`.

use std::path::PathBuf;

use directories::ProjectDirs;

/// Resolved XDG directories, brand-stamped for ttymap.
#[derive(Clone, Debug)]
pub struct AppDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    /// `state_dir()` is Linux-only in the `directories` crate; on
    /// macOS / Windows it returns `None`. The pre-#362 logging code
    /// fell back to `data_local_dir()` in that case — that policy
    /// lives here now so every consumer sees a single `state` path
    /// regardless of platform.
    pub state: PathBuf,
}

impl AppDirs {
    /// Resolve the per-user XDG dirs for the `ttymap` brand. Returns
    /// `None` when `directories::ProjectDirs::from` can't find a home
    /// directory (rare: e.g. `HOME` unset in CI sandboxes).
    pub fn resolve() -> Option<Self> {
        let d = ProjectDirs::from("", "", "ttymap")?;
        let state = d
            .state_dir()
            .unwrap_or_else(|| d.data_local_dir())
            .to_path_buf();
        Some(Self {
            config: d.config_dir().to_path_buf(),
            data: d.data_dir().to_path_buf(),
            cache: d.cache_dir().to_path_buf(),
            state,
        })
    }
}
