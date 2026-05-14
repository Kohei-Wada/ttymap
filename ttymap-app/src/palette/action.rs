//! `PaletteAction` enum — what a [`PaletteProvider`] tells the palette
//! to do after the user activates an item or cancels.
//!
//! Mirrors `map::MapAction` in role: a closed verb vocabulary owned by
//! the palette subsystem. `PaletteComponent::apply_action` is the
//! single consumer; providers (Rust + Lua bridge) only emit. Splitting
//! it out of the trait file keeps the public verb set discoverable
//! independent of the trait surface.
//!
//! [`PaletteProvider`]: super::provider::PaletteProvider

use crate::UserCommand;
use crate::compositor::Component;

use super::provider::PaletteProvider;

/// What a provider wants the host to do when the user activates an
/// item. Translated by the palette Component into the equivalent
/// `win.*` calls.
///
/// Lua-side reachability of each variant:
///
/// | Variant            | Rust provider | Lua provider                     |
/// |--------------------|---------------|----------------------------------|
/// | `Close`            | return        | `nil` / `{close=true}` (default) |
/// | `Run(msgs)`        | return        | side channel (`ttymap.map:*`)    |
/// | `Push(component)`  | return        | side channel (`api.card.open`) |
/// | `SwitchProvider`   | return        | `{switch = spec}` from execute   |
///
/// `Run` and `Push` are reachable from Lua only via the host
/// `intent_tx` / `push_tx` channels — Lua cannot construct an
/// `UserCommand` enum or a `Box<dyn Component>` directly. `SwitchProvider`
/// has no equivalent side channel (provider swap on a *running*
/// palette must be atomic) so it's the one structural verb exposed in
/// the Lua return shape.
pub enum PaletteAction {
    /// Close the palette with no side effect.
    Close,
    /// Close the palette and dispatch these messages.
    Run(Vec<UserCommand>),
    /// Close the palette and push `component` onto the compositor.
    /// Always stacks new — no Rust-side dedup. A plugin that wants
    /// "close existing on re-select" implements that itself.
    Push(Box<dyn Component>),
    /// Swap the palette's provider in place — sub-mode transition
    /// (e.g. "Theme"). Palette stays open.
    SwitchProvider(Box<dyn PaletteProvider>),
}
