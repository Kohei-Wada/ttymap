//! CLI subcommands. Each variant of [`Command`] has a corresponding
//! submodule with a `run()` entry point, so `main.rs` stays focused
//! on CLI parsing + process-level setup.
//!
//! Adding a subcommand:
//!   1. Create `src/commands/<name>.rs` with `pub fn run(...) -> Result<(), Box<dyn std::error::Error>>`
//!   2. Add a `pub mod <name>;` line below.
//!   3. Add the variant to [`Command`] and a match arm in [`Command::run`].

use clap::Subcommand;

pub mod api_info;
pub mod snap;

#[derive(Subcommand)]
pub enum Command {
    /// Render a single map snapshot as ANSI text (headless).
    #[command(alias = "snapshot")]
    Snap(snap::SnapArgs),
    /// Dump the machine-readable Lua API spec as JSON.
    #[command(name = "api-info")]
    ApiInfo(api_info::ApiInfoArgs),
}

impl Command {
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Snap(args) => snap::run(args),
            Self::ApiInfo(args) => api_info::run(args),
        }
    }
}

/// Resolve and cache the layered runtime path. Aborts the process
/// with a one-line message if every layer is missing — there's no
/// graceful degradation for callers that need init.lua to load.
///
/// Each subcommand calls this itself (the headless `snap` wants
/// init.lua-tunable config; `api-info` doesn't and skips the call).
/// Same goes for the interactive event loop. Keeping the call inside
/// the consumer matches "each subcommand owns its setup" — the binary
/// entry is just a dispatcher.
pub fn init_runtime_or_exit() {
    match crate::lua::resolve_runtime_path() {
        Ok(p) => crate::lua::set_runtime_path(p),
        Err(e) => {
            eprintln!("ttymap: {}", e);
            std::process::exit(1);
        }
    }
}
