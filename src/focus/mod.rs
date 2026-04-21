//! Focus state machine + focus-claiming surface contract.
//!
//! Two concerns live here, split across submodules:
//!
//! - [`surface`] — the [`FocusSurface`] trait and the small types its
//!   contract names ([`Effect`], [`SurfaceId`], [`SurfaceCtx`]).
//!   Anything that can claim keyboard focus implements this.
//! - This module — [`Focus`] (the state machine's state) and
//!   [`FocusManager`] (its owner, plus the plugin registry and the
//!   [`BackgroundResponder`]).
//!
//! External callers import freely via `crate::focus::*` — the
//! submodule split is a file-organisation detail.
//!
//! **No `None` for the router**: when no modal claims focus,
//! [`FocusManager::focused_surface_mut`] returns the
//! [`BackgroundResponder`] — it is itself a [`FocusSurface`] (always
//! visible, handles global keys). The router stays a one-call
//! dispatcher and never special-cases the background.
//!
//! **Surfaces are opaque ids**: every focusable surface — palette,
//! search, wiki, help, any future modal — is a [`Plugin`] in the
//! registry, addressed by [`SurfaceId`]. The manager is symmetric
//! across them; the only special surface is the background, which
//! has its own `Focus::Background` variant precisely because it is
//! the resting state, not a destination.
//!
//! [`Plugin`]: crate::plugin::Plugin

pub mod surface;

pub use surface::{Effect, FocusSurface, SurfaceCtx, SurfaceId};

use crate::app_command::AppCommand;
use crate::background::BackgroundResponder;
use crate::plugin::PluginRegistry;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    /// Default state — no surface has claimed input. The
    /// [`BackgroundResponder`] handles keys.
    #[default]
    Background,
    /// A modal surface (focused plugin or any future modal) has
    /// claimed input.
    Modal(SurfaceId),
}

/// Owns plugins + background + focus state. Single point of authority
/// for "who has keyboard focus and what surfaces exist". `prev`
/// restores the focus the previous claimer had instead of always
/// dropping to background.
pub struct FocusManager {
    current: Focus,
    prev: Focus,
    widgets: PluginRegistry,
    background: BackgroundResponder,
}

impl FocusManager {
    /// Construct from a pre-built plugin registry + background
    /// responder. Both are wired at the composition root (`App::new`).
    /// The palette is registered inside `widgets` like every other
    /// plugin.
    pub fn new(widgets: PluginRegistry, background: BackgroundResponder) -> Self {
        Self {
            current: Focus::Background,
            prev: Focus::Background,
            widgets,
            background,
        }
    }

    /// **The router's primary API**: return the surface that should
    /// receive the next key event. Always returns *some* surface —
    /// when no modal claims focus, the background responder is
    /// returned.
    pub fn focused_surface_mut(&mut self) -> &mut dyn FocusSurface {
        match &self.current {
            Focus::Background => &mut self.background,
            Focus::Modal(id) => {
                let id = id.clone();
                if let Some(p) = self.widgets.get_mut(id.as_ref()) {
                    p
                } else {
                    // Modal id refers to a plugin that is no longer
                    // registered (defensive — the registry shouldn't
                    // shrink at runtime). Fall back to background so
                    // the router still has a surface to talk to.
                    &mut self.background
                }
            }
        }
    }

    /// Immutable counterpart of [`focused_surface_mut`] — used by the
    /// UI layer to query non-mutating properties (today: footer
    /// hints) of whichever surface holds focus, without special-
    /// casing background.
    pub fn focused_surface(&self) -> &dyn FocusSurface {
        match &self.current {
            Focus::Background => &self.background,
            Focus::Modal(id) => self
                .widgets
                .get(id.as_ref())
                .map(|p| p as &dyn FocusSurface)
                .unwrap_or(&self.background),
        }
    }

    /// Release the currently-held modal focus (if any). Called by the
    /// router after `handle_key` when the surface reports
    /// `is_visible() == false`. No-op for `Focus::Background`.
    pub fn release_focused(&mut self) {
        if matches!(&self.current, Focus::Modal(_)) {
            self.release();
        }
    }

    // ── State queries ────────────────────────────────────────────────

    pub fn current(&self) -> &Focus {
        &self.current
    }

    // ── Field accessors (for draw, async polling) ────────────────────

    pub fn widgets(&self) -> &PluginRegistry {
        &self.widgets
    }

    /// Background responder — used by the router to fall through
    /// global keys (activation keys / keymap fallback) when the
    /// currently-focused modal surface returns [`Effect::Pass`]. The
    /// background is also the focused surface when no modal claims
    /// focus, so a key handed to it twice (focused-modal Pass → here)
    /// can't happen: this accessor is only useful in the *modal-Pass*
    /// branch.
    pub fn background_mut(&mut self) -> &mut BackgroundResponder {
        &mut self.background
    }

    // ── Workflow API ─────────────────────────────────────────────────

    /// Open / activate the named surface and transfer focus to it.
    /// Single entry point invoked by the router on `Effect::Open(id)`.
    ///
    /// Toggle-off (re-pressing the activation key while the plugin
    /// holds focus) is **the surface's own responsibility**: every
    /// modal in the codebase consumes its activation key and
    /// self-closes (palette / search → `Esc`; wiki → second `i`;
    /// help → any key). That means `Effect::Open(id)` never reaches
    /// `open()` while `id` is already the focused surface, so this
    /// path only handles the "fresh activation" case.
    ///
    /// Behaviour by id:
    /// - **registered plugin**: bring to front, call its `activate`
    ///   hook with the supplied `ctx` snapshot (geo-aware plugins
    ///   read `ctx.center`, palette reads `ctx.theme_id`), take focus
    ///   iff `wants_focus()` is true (headless plugins like `here`
    ///   don't steal focus).
    /// - **unknown id**: no-op (defensive — registries shouldn't
    ///   shrink at runtime).
    pub fn open(&mut self, id: SurfaceId, ctx: SurfaceCtx) {
        self.widgets.bring_to_front(id.as_ref());
        let wants_focus = if let Some(w) = self.widgets.get_mut(id.as_ref()) {
            w.activate(ctx);
            w.wants_focus()
        } else {
            return;
        };
        if wants_focus {
            self.deactivate_current(Some(id.as_ref()));
            self.transition_to(Focus::Modal(id));
        }
    }

    /// Cycle focus to the next (or previous) visible plugin, wrapping
    /// through Background. Returns `true` if focus moved.
    /// Background → visible[0] → … → visible[last] → Background
    /// (reverse swaps the ends).
    ///
    /// Visibility-based filtering keeps the palette out naturally:
    /// the palette is only visible when opened, and once opened it
    /// already holds focus, so cycle is never invoked from
    /// `Background` while palette is also visible.
    pub fn cycle(&mut self, forward: bool) -> bool {
        let visible: Vec<String> = self
            .widgets
            .iter()
            .filter(|w| w.is_visible())
            .map(|w| w.tag().to_string())
            .collect();

        if visible.is_empty() {
            return false;
        }

        let next: Option<String> = match &self.current {
            Focus::Background => Some(if forward {
                visible.first().unwrap().clone()
            } else {
                visible.last().unwrap().clone()
            }),
            Focus::Modal(cur) => {
                let cur_str = cur.as_ref();
                match visible.iter().position(|t| t == cur_str) {
                    Some(i) if forward => {
                        if i + 1 < visible.len() {
                            Some(visible[i + 1].clone())
                        } else {
                            None
                        }
                    }
                    Some(i) => {
                        if i > 0 {
                            Some(visible[i - 1].clone())
                        } else {
                            None
                        }
                    }
                    None => Some(if forward {
                        visible.first().unwrap().clone()
                    } else {
                        visible.last().unwrap().clone()
                    }),
                }
            }
        };

        self.deactivate_current(None);
        self.transition_to(match next {
            Some(tag) => Focus::Modal(tag.into()),
            None => Focus::Background,
        });
        true
    }

    /// Advance every plugin's async work by one tick. If multiple
    /// plugins produced a `AppCommand` this tick, the latest wins.
    pub fn poll_widgets(&mut self) -> Option<AppCommand> {
        let mut async_cmd: Option<AppCommand> = None;
        for w in self.widgets.iter_mut() {
            w.poll();
            if let Some(cmd) = w.pending_command() {
                async_cmd = Some(cmd);
            }
        }
        async_cmd
    }

    // ── Internals ────────────────────────────────────────────────────

    /// Restore the remembered predecessor (or background if none).
    fn release(&mut self) {
        self.current = std::mem::replace(&mut self.prev, Focus::Background);
    }

    fn transition_to(&mut self, new: Focus) {
        if new != self.current {
            self.prev = std::mem::replace(&mut self.current, new);
        }
    }

    fn deactivate_current(&mut self, keep_id: Option<&str>) {
        if let Focus::Modal(prev) = &self.current {
            let prev_str = prev.clone();
            if keep_id != Some(prev_str.as_ref())
                && let Some(p) = self.widgets.get_mut(prev_str.as_ref())
            {
                p.deactivate();
            }
        }
    }
}
