//! `FocusManager` — owns every surface that can handle keys (palette,
//! plugin registry, background responder) and tracks which one
//! currently has keyboard focus. The router asks
//! [`focused_surface_mut`] to find out where to send keys; everything
//! else (palette open, plugin activation, focus cycle, async polling)
//! is a method on this type so the focus / widgets / palette state
//! stay consistent without external coordination.
//!
//! **No `None` for the router**: when no modal claims focus,
//! `focused_surface_mut` returns the [`BackgroundResponder`] —
//! it is itself a [`FocusSurface`] (always visible, handles global
//! keys). The router stays a one-call dispatcher and never special-
//! cases the background.
//!
//! **Surfaces are opaque ids**: the manager does not distinguish
//! palette from plugin from any future modal (dialog, notification
//! tray); they all flow through the `Modal(SurfaceId)` variant. The
//! `wants_focus` gating policy lives at the call site (e.g.
//! `activate_plugin`), not inside the focus state machine.

use std::borrow::Cow;

use crate::app_command::{AppCommand, FocusSurface};
use crate::background::BackgroundResponder;
use crate::color_palette::ThemeId;
use crate::geo::LonLat;
use crate::plugin::PluginRegistry;
use crate::ui::palette::{self, CommandPalette};

/// Identifier for a focus-claiming surface. `palette`, plugin tags
/// (`search`, `wiki`, …), and any future modal share the same shape.
pub type SurfaceId = Cow<'static, str>;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    /// Default state — no surface has claimed input. The
    /// [`BackgroundResponder`] handles keys.
    #[default]
    Background,
    /// A modal surface (palette, focused plugin, or any future modal)
    /// has claimed input.
    Modal(SurfaceId),
}

impl Focus {
    /// Whether the named surface is the current focus owner.
    pub fn is_modal(&self, id: &str) -> bool {
        matches!(self, Focus::Modal(t) if t == id)
    }
}

/// Owns palette + plugins + background + focus state. Single point of
/// authority for "who has keyboard focus and what surfaces exist".
/// `prev` restores the focus the previous claimer had instead of
/// always dropping to background.
pub struct FocusManager {
    current: Focus,
    prev: Focus,
    palette: CommandPalette,
    widgets: PluginRegistry,
    background: BackgroundResponder,
}

impl FocusManager {
    /// Construct from pre-built palette + plugin registry +
    /// background responder. All three are wired at the composition
    /// root (`App::new`).
    pub fn new(
        palette: CommandPalette,
        widgets: PluginRegistry,
        background: BackgroundResponder,
    ) -> Self {
        Self {
            current: Focus::Background,
            prev: Focus::Background,
            palette,
            widgets,
            background,
        }
    }

    /// **The router's primary API**: return the surface that should
    /// receive the next key event. Always `Some` — when no modal
    /// claims focus, the background responder is returned.
    pub fn focused_surface_mut(&mut self) -> &mut dyn FocusSurface {
        match &self.current {
            Focus::Background => &mut self.background,
            Focus::Modal(id) => {
                let id = id.clone();
                if id == palette::SURFACE_ID {
                    &mut self.palette
                } else if let Some(p) = self.widgets.get_mut(id.as_ref()) {
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

    pub fn is_modal(&self, id: &str) -> bool {
        self.current.is_modal(id)
    }

    // ── Field accessors (for draw, async polling) ────────────────────

    pub fn widgets(&self) -> &PluginRegistry {
        &self.widgets
    }

    pub fn palette(&self) -> &CommandPalette {
        &self.palette
    }

    // ── Workflow API ─────────────────────────────────────────────────

    /// Open the command palette with the default provider and take
    /// focus. The palette's provider needs `widgets` (for activation
    /// lists), `keymap` (for key hints — pulled from the background
    /// responder), and `theme_id` (for the theme picker entry).
    pub fn open_palette(&mut self, theme_id: ThemeId) {
        self.palette
            .activate(&self.widgets, self.background.keymap(), theme_id);
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
        let wants_focus = if let Some(w) = self.widgets.get_mut(tag) {
            w.activate(center);
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

    fn release_if_holding(&mut self, id: &str) {
        if self.current.is_modal(id) {
            self.release();
        }
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
