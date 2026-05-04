//! Core layer — engine state + GoF Receiver for [`crate::UserCommand`].
//!
//! Owns the state that mutates in response to commands (map, lua,
//! compositor, theme, sidebar, overlay sink, cursor). The
//! [`crate::app::App`] type sits **above** core as the loop driver:
//! it drains the [`crate::app::AppEvent`] bus, ratatui-draws each
//! frame, and forwards commands to [`Dispatcher`].
//!
//! `core/` is **ratatui-free** at the import-graph level (modulo
//! the `Component::render` method's signature, which still
//! transitively names a front type — see issue #212 Phase 2 notes).
//! That layering invariant is the whole reason this directory
//! exists rather than the dispatcher living under `app/`.
//!
//! Phase 4 of GitHub issue #212 (architectural cleanup: split
//! front/core).

pub mod compositor;
mod dispatcher;
pub mod map;
mod overlay;
mod sidebar;

pub(crate) use dispatcher::Dispatcher;
