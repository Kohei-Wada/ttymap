//! Activation primitives — what `register_keybind` /
//! `register_palette_command` produce at registration time.
//!
//! Pure data + factory closures; no behaviour. The compositor's
//! [`super::base::BaseLayer`] consumes [`Activation`]s; the palette
//! installer consumes [`PaletteEntry`]s. Each entry is just a row in
//! a flat registry; the host has no notion of "plugin" — that's a
//! Lua-side conventional grouping (one .lua file's worth of
//! `register_*` calls).

use std::rc::Rc;

use crossterm::event::{KeyCode, KeyModifiers};

use super::component::{Component, Context};

/// Factory closure producing a fresh [`Component`] when the user
/// activates the corresponding surface. Receives a [`Context`]
/// snapshot so plugins that read app-level state at activation time
/// (e.g. palette seeds its "(current)" theme hint from `theme_id`)
/// can do so without a separate lifecycle hook.
///
/// Returns `None` when the factory wants to skip the push entirely
/// — used by Lua plugins whose activation callback returned a falsy
/// value, signalling "I read my state and decided not to open this
/// time".
///
/// Stored as `Rc<dyn Fn>` rather than `Box<dyn Fn>` so the registry
/// can clone the factory out under a short borrow and invoke it
/// after the borrow drops — letting a Lua plugin's activation
/// callback safely call `:remove()` on its own
/// `KeybindHandle` / `PaletteCommandHandle` without `RefCell`
/// re-entry panicking. The clone is a cheap reference-count bump.
pub type SpawnComponent = Rc<dyn Fn(&Context) -> Option<Box<dyn Component>>>;

/// One activation entry — "when this key is pressed while nothing
/// modal is above the bottom layer, invoke `spawn` and push the
/// result". Collected by [`crate::lua::Registrar`] at plugin-load
/// time and consumed by [`super::BaseLayer`] at startup.
pub struct Activation {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub spawn: SpawnComponent,
}

/// Palette entry description. Selection always pushes a fresh
/// component on the stack — there's no toggle/spawn distinction now
/// that the compositor doesn't dedup. A caller that wants "close on
/// re-select" closes itself in its own `handle_key`.
///
/// `hint` is the keybind string the entry is also bound to (e.g.
/// `"w"`); empty string when the entry is palette-only. Used by the
/// help cheatsheet and the footer-hint slot to surface the binding
/// alongside the label.
///
/// `Clone` is a cheap `Rc` bump on `spawn` plus two `String` clones,
/// taken when a [`PaletteIndex`] hands a snapshot to the palette.
#[derive(Clone)]
pub struct PaletteEntry {
    pub label: String,
    pub hint: String,
    pub spawn: SpawnComponent,
}

/// Read-only lookup of keybind activations. Implemented by whatever
/// store actually owns the activation list (today the Lua-side
/// registry; trivially extensible to in-Rust built-ins or future
/// runtime sources). [`super::BaseLayer`] depends only on this
/// trait, so the compositor stays unaware of the Lua subsystem.
pub trait ActivationIndex {
    /// Spawn factory for a `(code, modifiers)` press, if any plugin
    /// has registered an activation against it. `&self` because
    /// dispatch is read-only — internal interior mutability (e.g.
    /// `RefCell`) is the implementor's concern.
    fn find_spawn(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<SpawnComponent>;
}

/// Read-only view of the palette command list. Implemented by
/// whatever store owns the entries; [`super::PaletteEntry`] is the
/// transfer shape. The palette's `CommandProvider` depends only on
/// this trait so the palette UI stays unaware of the Lua subsystem.
pub trait PaletteIndex {
    /// Snapshot of `(id, entry)` pairs in registration order. Taken
    /// once per palette open; mutations after the snapshot land in
    /// later opens.
    fn entries(&self) -> Vec<(u64, PaletteEntry)>;
    /// Spawn factory for a palette entry by its handle ID. Returns
    /// `None` when the entry has been removed since the snapshot was
    /// built — `CommandProvider::execute` treats that as a silent
    /// no-op so a stale selection can't dispatch against a phantom
    /// factory.
    fn entry_spawn(&self, id: u64) -> Option<SpawnComponent>;
}
