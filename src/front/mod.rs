//! Front layer — UI / IO shell that sits above [`crate::core`].
//!
//! Houses things that are presentation-bound: ratatui-aware
//! components, palette / picker UI, CLI subcommand entry points.
//! [`crate::app::App`] (the loop driver) lives outside `front/`
//! today but conceptually belongs to this layer; it stays at
//! `src/app/` for now to limit churn (see issue #212).
//!
//! Phase 4 of GitHub issue #212 (architectural cleanup: split
//! front/core).

pub mod cli;
pub mod palette;
pub mod theme;
