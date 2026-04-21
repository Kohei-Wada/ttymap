//! `Focus` — single source of truth for which surface has exclusive
//! keyboard focus. Read by the dispatcher and layout code; transitions
//! happen via [`FocusManager::on`] which interprets [`FocusEvent`]s
//! emitted by `UiState`.
//!
//! **Event-driven, not commanded**: callers don't tell focus *what* to
//! do (`take`, `release`), they tell focus *what happened* in the rest
//! of the UI (`Claimed(id)`, `Released(id)`) and focus decides how to
//! react. The transition rules (auto-release on close, prev-slot
//! restoration) live in one place — here — instead of scattered.
//!
//! **Surfaces are opaque ids**: the manager does not distinguish
//! palette from plugin from any future modal (dialog, notification
//! tray); they all flow through the same `Modal(SurfaceId)` variant
//! and `Claimed/Released` events. The `wants_focus` gating policy
//! lives at the call site (e.g. `UiState::activate_plugin`), not here.

use std::borrow::Cow;

use crate::plugin::PluginRegistry;

/// Identifier for a focus-claiming surface. `palette`, plugin tags
/// (`search`, `wiki`, …), and any future modal share the same shape.
pub type SurfaceId = Cow<'static, str>;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    /// Default state — no surface has claimed input. The
    /// [`BackgroundResponder`](crate::ui::router::background::BackgroundResponder)
    /// handles keys.
    #[default]
    Background,
    /// A modal surface (palette, focused plugin, or any future modal)
    /// has claimed input. The router delivers keys to it before
    /// falling through to the background.
    Modal(SurfaceId),
}

impl Focus {
    /// Whether the named surface is the current focus owner. Works
    /// for any modal id — palette, plugin tag, or other.
    pub fn is_modal(&self, id: &str) -> bool {
        matches!(self, Focus::Modal(t) if t == id)
    }
}

/// Events the rest of the UI emits at focus-relevant moments. The
/// manager interprets each event against its current state to decide
/// the transition.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusEvent {
    /// Surface with this id is asking for focus. The caller is
    /// responsible for any "wants focus" gating before emitting (the
    /// manager unconditionally honours the claim).
    Claimed(SurfaceId),
    /// Surface with this id closed (key handler dropped its
    /// visibility, or the user toggled it off). If focus is on this
    /// surface, the manager releases it.
    Released(SurfaceId),
}

/// Coordinates focus transitions. Inner `Focus` is private so every
/// transition goes through [`on`] (or [`cycle`]), letting the
/// manager keep the `prev` slot in sync. `prev` restores the focus
/// the previous claimer had instead of always dropping to background.
#[derive(Default)]
pub struct FocusManager {
    current: Focus,
    prev: Focus,
}

impl FocusManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only access to the current focus for pattern matching and
    /// equality checks.
    pub fn current(&self) -> &Focus {
        &self.current
    }

    /// Whether the named surface is the current focus owner. Sugar
    /// over [`Focus::is_modal`].
    pub fn is_modal(&self, id: &str) -> bool {
        self.current.is_modal(id)
    }

    /// React to a UI state change. See [`FocusEvent`] for the vocab.
    pub fn on(&mut self, event: FocusEvent, widgets: &mut PluginRegistry) {
        match event {
            FocusEvent::Claimed(id) => {
                self.deactivate_current(widgets, Some(&id));
                self.transition_to(Focus::Modal(id));
            }
            FocusEvent::Released(id) => {
                if self.current.is_modal(&id) {
                    self.release();
                }
            }
        }
    }

    /// Cycle focus to the next (or previous) visible plugin, wrapping
    /// through Background. Tab is an explicit user intent, not a
    /// reactive transition, so it stays a dedicated method instead of
    /// riding `on`. Returns `true` if focus moved.
    /// Background → visible[0] → … → visible[last] → Background
    /// (reverse swaps the ends).
    pub fn cycle(&mut self, widgets: &mut PluginRegistry, forward: bool) -> bool {
        // Non-plugin modals (palette, future dialogs) are outside the
        // plugin cycle — user must close them first. Detected by
        // looking up the current id in the plugin registry; if it's
        // not there, it's a non-plugin modal. (palette.handle_key
        // also consumes Tab, so this branch is mostly defensive.)
        if let Focus::Modal(id) = &self.current
            && widgets.get(id.as_ref()).is_none()
        {
            return false;
        }

        let visible: Vec<String> = widgets
            .iter()
            .filter(|w| w.visible())
            .map(|w| w.tag().to_string())
            .collect();

        if visible.is_empty() {
            return false;
        }

        let next: Option<String> = match &self.current {
            // From Background, enter at the appropriate end of the list.
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
                            None // past last → Background
                        }
                    }
                    Some(i) => {
                        if i > 0 {
                            Some(visible[i - 1].clone())
                        } else {
                            None // before first → Background
                        }
                    }
                    // Current focus not visible — enter the list at the edge.
                    None => Some(if forward {
                        visible.first().unwrap().clone()
                    } else {
                        visible.last().unwrap().clone()
                    }),
                }
            }
        };

        self.deactivate_current(widgets, None);
        self.transition_to(match next {
            Some(tag) => Focus::Modal(tag.into()),
            None => Focus::Background,
        });
        true
    }

    // ── Internals ─────────────────────────────────────────────────

    /// Restore the remembered predecessor (or background if none).
    fn release(&mut self) {
        self.current = std::mem::replace(&mut self.prev, Focus::Background);
    }

    /// Record a transition, pushing the outgoing focus into `prev`.
    /// No-op when the target already matches current (guards against
    /// self-reactivation clobbering a useful `prev`).
    fn transition_to(&mut self, new: Focus) {
        if new != self.current {
            self.prev = std::mem::replace(&mut self.current, new);
        }
    }

    /// Run `deactivate` on the currently-focused plugin unless the
    /// caller is about to re-activate the same one (toggle case).
    /// Non-plugin modals (e.g. palette) aren't in the registry so the
    /// `widgets.get_mut` lookup returns `None` and nothing happens —
    /// they self-clean through their own visibility flow.
    fn deactivate_current(&self, widgets: &mut PluginRegistry, keep_id: Option<&str>) {
        if let Focus::Modal(prev) = &self.current
            && keep_id != Some(prev.as_ref())
            && let Some(p) = widgets.get_mut(prev.as_ref())
        {
            p.deactivate();
        }
    }
}
