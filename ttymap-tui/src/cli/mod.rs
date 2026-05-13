//! CLI subcommands. Each variant of [`Command`] has a corresponding
//! submodule with a `run()` entry point, so `main.rs` stays focused
//! on CLI parsing + process-level setup.
//!
//! Adding a subcommand:
//!   1. Create `src/commands/<name>.rs` with `pub fn run(...) -> Result<(), Box<dyn std::error::Error>>`
//!   2. Add a `pub mod <name>;` line below.
//!   3. Add the variant to [`Command`] and a match arm in [`Command::run`].

use clap::Subcommand;

pub mod snap;

#[derive(Subcommand)]
pub enum Command {
    /// Render a single map snapshot as ANSI text (headless).
    #[command(alias = "snapshot")]
    Snap(snap::SnapArgs),

    /// Run as the headless engine subprocess. Spawned by the TUI
    /// parent over a stdin/stdout IPC pipe; rarely useful from a
    /// shell directly (the worker expects bincode-framed
    /// `EngineCommand`s on stdin). See #348.
    EngineWorker,
}

impl Command {
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            Self::Snap(args) => snap::run(args),
            Self::EngineWorker => ttymap_engine::run_as_subprocess(),
        }
    }
}
