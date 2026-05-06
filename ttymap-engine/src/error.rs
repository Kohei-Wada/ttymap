//! Engine-wide error type returned across the public boundary.
//!
//! Internal hot paths (decoder zigzag, renderer math, Braille bit
//! packing, …) keep their `unwrap`s — surfacing those would just add
//! an error path on every frame for invariants that cannot recover.
//! See `docs/design.md` "Error boundary policy" for the full cut.

use std::io;
use std::path::PathBuf;

use crate::shared::http::FetchError;

/// Error returned by every public engine API. The variants are
/// classified by the boundary that produces them so callers can
/// decide policy (retry / surface to UI / fail fast) without
/// downcasting.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Failed to create the on-disk tile cache directory. The user
    /// can usually fix this (perms / disk full) — surface the path
    /// in the error.
    #[error("create cache directory {path}: {source}")]
    CacheDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// `reqwest::Client` builder failed at construction. In
    /// practice this almost never fires (no I/O happens at build
    /// time), but it's reachable so we route it instead of
    /// panicking on the public boundary.
    #[error("HTTP client init: {0}")]
    HttpInit(#[source] reqwest::Error),

    /// Per-request HTTP fetch error from `shared::http`. Kept as a
    /// distinct variant so narrow callers can keep using
    /// `Result<_, FetchError>` and have it auto-convert via `?`.
    #[error(transparent)]
    Fetch(#[from] FetchError),
}
