//! CLI subcommands. Each variant of [`Command`] has a corresponding
//! submodule with a `run()` entry point, so `main.rs` stays focused
//! on CLI parsing + process-level setup.
//!
//! Adding a subcommand:
//!   1. Create `src/commands/<name>.rs` with `pub fn run(...) -> io::Result<()>`
//!   2. Add a `pub mod <name>;` line below.
//!   3. Add the variant to [`Command`] and a match arm in [`Command::run`].

use std::io;

use clap::Subcommand;

pub mod clear_cache;

#[derive(Subcommand)]
pub enum Command {
    /// Clear the disk tile cache (~/.cache/ttymap/).
    ClearCache,
}

impl Command {
    pub fn run(self) -> io::Result<()> {
        match self {
            Self::ClearCache => clear_cache::run(),
        }
    }
}
