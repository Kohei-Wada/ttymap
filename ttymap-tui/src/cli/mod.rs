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

    /// Whether this subcommand needs `lua::resolve_runtime_path` /
    /// `set_runtime_path` to have run before [`Self::run`]. `snap`
    /// reads init.lua for config knobs, so it does. `api-info` is a
    /// pure metadata dump from compile-time `&'static` data — it must
    /// keep working on a freshly-installed system that has nothing in
    /// `~/.config/ttymap` or `~/.local/share/ttymap` yet.
    pub fn needs_runtime(&self) -> bool {
        match self {
            Self::Snap(_) => true,
            Self::ApiInfo(_) => false,
        }
    }
}
