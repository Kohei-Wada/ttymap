//! `FocusManager` — owns every focusable surface (the command palette
//! and every plugin) and tracks which one currently has keyboard
//! focus. The router asks `focused_surface_mut` to find out who to
//! deliver a key event to; everything else (cycling, activation,
//! palette open) is a method on this type so the focus / widget /
//! palette state stay consistent without external coordination.
//!
//! **Surfaces are opaque ids**: the manager does not distinguish
//! palette from plugin from any future modal (dialog, notification
//! tray); they all flow through the same `Modal(SurfaceId)` variant.
//! The `wants_focus` gating policy lives at the call site (e.g.
//! `activate_plugin`), not inside the focus state machine.

use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app_command::{AppCommand, Effect, FocusSurface, SurfaceCtx};
use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::keymap::KeyMap;
use crate::plugin::PluginRegistry;
use crate::ui::palette::{self, CommandPalette};

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
    /// Whether the named surface is the current focus owner.
    pub fn is_modal(&self, id: &str) -> bool {
        matches!(self, Focus::Modal(t) if t == id)
    }
}

/// Owns palette + plugins + focus state. Single point of authority
/// for "who has keyboard focus and what surfaces exist". `prev`
/// restores the focus the previous claimer had instead of always
/// dropping to background.
pub struct FocusManager {
    current: Focus,
    prev: Focus,
    palette: CommandPalette,
    widgets: PluginRegistry,
}

impl FocusManager {
    /// Construct from a pre-built palette + plugin registry. Both
    /// are wired at the composition root (`App::new`) so the focus
    /// manager doesn't need to know how to create them.
    pub fn new(palette: CommandPalette, widgets: PluginRegistry) -> Self {
        Self {
            current: Focus::Background,
            prev: Focus::Background,
            palette,
            widgets,
        }
    }

    // ── State queries ────────────────────────────────────────────────

    pub fn current(&self) -> &Focus {
        &self.current
    }

    pub fn is_modal(&self, id: &str) -> bool {
        self.current.is_modal(id)
    }

    // ── Field accessors (for draw, background-responder activation
    // lookup, async polling) ─────────────────────────────────────────

    pub fn widgets(&self) -> &PluginRegistry {
        &self.widgets
    }

    pub fn palette(&self) -> &CommandPalette {
        &self.palette
    }

    // ── The router's primary API ─────────────────────────────────────

    /// Hand a key to the focused surface. Applies the auto-release
    /// invariant (if `is_visible()` flips to false during `handle_key`,
    /// release focus). Returns `None` when no surface is focused —
    /// the caller should then fall through to the background responder.
    pub fn deliver_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: SurfaceCtx,
    ) -> Option<Effect> {
        let id = match &self.current {
            Focus::Background => return None,
            Focus::Modal(id) => id.clone(),
        };
        let (effect, still_visible) = {
            let surface = self.surface_mut(&id)?;
            let effect = surface.handle_key(code, modifiers, ctx);
            let still_visible = surface.is_visible();
            (effect, still_visible)
        };
        if !still_visible {
            self.release_if_holding(&id);
        }
        Some(effect)
    }

    // ── Workflow API ─────────────────────────────────────────────────

    /// Open the command palette with the default provider and take
    /// focus. Provider build needs `widgets` (for activation lists),
    /// `keymap` (for key hints), `theme_id` (for the theme picker).
    pub fn open_palette(&mut self, keymap: &KeyMap, theme_id: ThemeId) {
        self.palette.activate(&self.widgets, keymap, theme_id);
        self.transition_to(Focus::Modal(palette::SURFACE_ID.into()));
    }

    /// Drive a plugin through an activation request. Re-activating a
    /// currently-focused plugin toggles it off. Otherwise brings the
    /// plugin to the front, calls its `activate` hook, and takes focus
    /// iff `wants_focus` returns true (so headless plugins like `here`
    /// don't steal it).
    pub fn activate_plugin(&mut self, tag: &str, center: LonLat) {
        if self.is_modal(tag) {
            // Toggle-off: re-activating the currently-focused plugin closes it.
            if let Some(w) = self.widgets.get_mut(tag) {
                w.close();
            }
            self.release_if_holding(tag);
            return;
        }

        self.widgets.bring_to_front(tag);
        let ctx = SurfaceCtx { center };
        let wants_focus = if let Some(w) = self.widgets.get_mut(tag) {
            w.activate(ctx);
            w.wants_focus()
        } else {
            return;
        };
        if wants_focus {
            self.deactivate_current(Some(tag));
            self.transition_to(Focus::Modal(tag.to_string().into()));
        }
    }

    /// Cycle focus to the next (or previous) visible plugin, wrapping
    /// through Background. Returns `true` if focus moved.
    /// Background → visible[0] → … → visible[last] → Background
    /// (reverse swaps the ends). Non-plugin modals (palette, future
    /// dialogs) are outside the cycle — user must close them first.
    pub fn cycle(&mut self, forward: bool) -> bool {
        // Non-plugin modals are detected by absence from the registry.
        if let Focus::Modal(id) = &self.current
            && self.widgets.get(id.as_ref()).is_none()
        {
            return false;
        }

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

        self.deactivate_current(None);
        self.transition_to(match next {
            Some(tag) => Focus::Modal(tag.into()),
            None => Focus::Background,
        });
        true
    }

    /// Advance every plugin's async work by one tick. If multiple
    /// plugins produced a `AppCommand` this tick, the latest wins —
    /// only one AppCommand runs per tick to avoid cascading state
    /// changes within a single frame.
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

    fn surface_mut(&mut self, id: &SurfaceId) -> Option<&mut dyn FocusSurface> {
        if id == palette::SURFACE_ID {
            Some(&mut self.palette)
        } else {
            match self.widgets.get_mut(id.as_ref()) {
                Some(p) => Some(p),
                None => None,
            }
        }
    }

    /// Restore the remembered predecessor (or background if none).
    fn release(&mut self) {
        self.current = std::mem::replace(&mut self.prev, Focus::Background);
    }

    fn release_if_holding(&mut self, id: &str) {
        if self.current.is_modal(id) {
            self.release();
        }
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
