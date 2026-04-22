//! `FocusSurface` trait + the small types that its contract names —
//! `Effect`, `SurfaceId`, `SurfaceCtx`. Sibling of [`super::manager`]
//! concepts (the focus state machine): surfaces are *what* gets
//! focus; the manager tracks *which one* currently has it.
//!
//! `Effect::Run` wraps [`Vec<AppMsg>`](AppMsg), so this module depends
//! on `app::msg` (for the message vocabulary) but `App::dispatch`
//! itself does not depend on focus — the dispatcher never handles
//! `Effect`, only the `AppMsg`s it may carry.

use crate::app::AppMsg;
use crate::color_palette::ThemeId;
use crate::geo::LonLat;

/// Identifier for a focus-claiming surface (palette id, plugin tag,
/// any future modal). Lives here alongside [`Effect::Open`] so
/// `Effect` can name it without pulling in the manager.
pub type SurfaceId = std::borrow::Cow<'static, str>;

/// Outcome of handing a key to a [`FocusSurface`]. The router walks
/// responders (focused surface →
/// [`BackgroundResponder`](crate::background::BackgroundResponder)
/// — which is itself the focused surface when no modal claims focus).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Surface is not interested. The router treats this as a no-op
    /// (since `focused_surface_mut` always returns *some* surface,
    /// there is nowhere else to fall through to).
    Pass,
    /// Surface absorbed the key. No `AppMsg` to run.
    Consumed,
    /// Surface wants the host to run one or more messages. The
    /// router returns the vec to the caller, which hands each to
    /// `App::dispatch`. An empty vec is semantically equivalent to
    /// `Consumed` but is kept separate so "absorbed the key" and
    /// "absorbed the key and also emits N messages" stay
    /// syntactically distinct.
    Run(Vec<AppMsg>),
    /// Surface wants the focus manager to open / activate the named
    /// id and transfer focus to it. Router calls `focus.open(id, ctx)`
    /// which handles per-surface activation (palette setup, plugin
    /// `wants_focus` gating) + focus transition.
    Open(SurfaceId),
}

/// Read-only snapshot of app-level state passed into surface
/// lifecycle hooks ([`FocusSurface::handle_key`] and
/// [`Plugin::activate`](crate::plugin::Plugin::activate)). Lets a
/// surface read shared state it does not own — geo-aware plugins use
/// `center` for "act here" actions, the palette uses `theme_id` to
/// seed its theme-picker entry — without the dispatcher having to
/// know which surface needs what.
///
/// Kept `Copy` (every field is `Copy`) so call sites can hand it out
/// freely without lifetime gymnastics.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceCtx {
    pub center: LonLat,
    pub theme_id: ThemeId,
}

/// Uniform interface for anything that can claim focus. The router
/// hands a key event to whichever surface the
/// [`FocusManager`](super::FocusManager) currently identifies as
/// focused, then reads `is_visible` to detect "the surface closed
/// itself" and auto-release focus accordingly.
///
/// Implemented by [`CommandPalette`](crate::plugin::palette::CommandPalette)
/// and — via the `Plugin: FocusSurface` supertrait — by every
/// [`Plugin`](crate::plugin::Plugin).
pub trait FocusSurface {
    fn handle_key(
        &mut self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
        ctx: SurfaceCtx,
    ) -> Effect;

    /// Whether this surface is currently on screen / interactive.
    /// The router checks this after `handle_key` to detect "the
    /// surface closed itself" and auto-release focus; `FocusManager`
    /// also reads it for cycle eligibility (only visible surfaces
    /// participate in Tab cycle).
    ///
    /// Default `false` — the safe assumption for any new surface is
    /// "not yet shown". The only surface that opts in to "always
    /// visible" is [`BackgroundResponder`](crate::background::BackgroundResponder),
    /// which is never released and never appears in the cycle list
    /// (it's the resting state, not a destination).
    fn is_visible(&self) -> bool {
        false
    }

    /// Context-sensitive key hints for the footer, shown while this
    /// surface is the focused one. Lives on `FocusSurface` (not
    /// `Plugin`) so the [`BackgroundResponder`](crate::background::BackgroundResponder)
    /// can supply its own hint list through the same channel — the
    /// UI layer just calls `focused_surface().footer_hints()` and
    /// doesn't special-case background.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }
}
