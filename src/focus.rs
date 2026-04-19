//! `Focus` — single source of truth for which surface has exclusive
//! keyboard focus. Read by the dispatcher and layout code; transitions
//! happen via [`FocusManager::on`] which interprets [`FocusEvent`]s
//! emitted by `UiState` as palette / plugin state changes.
//!
//! **Event-driven, not commanded**: callers don't tell focus *what* to
//! do (`take`, `release`), they tell focus *what happened* in the rest
//! of the UI (`PaletteOpened`, `PluginActivated(tag)`, …) and focus
//! decides how to react. The transition rules (wants_focus gating,
//! auto-release on close, prev-slot restoration) live in one place —
//! here — instead of scattered across `UiState` methods.

use std::borrow::Cow;

use crate::plugin::PluginRegistry;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    #[default]
    Map,
    Plugin(Cow<'static, str>),
    /// Command palette — a builtin, not in `PluginRegistry`. Modelled
    /// as its own variant because the palette inherently coordinates
    /// across plugins and doesn't fit the self-contained widget
    /// contract `Plugin` imposes.
    Palette,
}

impl Focus {
    pub fn is_plugin(&self, tag: &str) -> bool {
        matches!(self, Focus::Plugin(t) if t == tag)
    }
}

/// Events the rest of the UI emits at focus-relevant moments. The
/// manager interprets each event against its current state and the
/// plugin registry to decide the transition.
pub enum FocusEvent {
    /// Palette's provider was activated — it is now modal and visible.
    PaletteOpened,
    /// Palette's `is_visible()` flipped to false (Esc / Enter / item
    /// picked). If focus is on the palette, releases it.
    PaletteClosed,
    /// Plugin with this tag just had `activate()` called and is ready
    /// to run. Focus takes it iff the plugin's `wants_focus()` is true.
    PluginActivated(String),
    /// Plugin with this tag closed (either `handle_key` dropped
    /// `visible()` or it was toggled off). If focus is on this plugin,
    /// releases it.
    PluginClosed(String),
}

/// Coordinates focus transitions. Inner `Focus` is private so every
/// transition goes through [`on`] (or [`cycle`]), letting the
/// manager keep the `prev` slot in sync. `prev` is how `release`
/// restores the focus a plugin held before the current one grabbed it
/// instead of always dropping back to the map.
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

    pub fn is_plugin(&self, tag: &str) -> bool {
        self.current.is_plugin(tag)
    }

    /// React to a UI state change. See [`FocusEvent`] for the vocab.
    pub fn on(&mut self, event: FocusEvent, widgets: &mut PluginRegistry) {
        match event {
            FocusEvent::PaletteOpened => {
                self.deactivate_current(widgets, None);
                self.transition_to(Focus::Palette);
            }
            FocusEvent::PaletteClosed => {
                if matches!(self.current, Focus::Palette) {
                    self.release();
                }
            }
            FocusEvent::PluginActivated(tag) => {
                let wants_focus = widgets.get(&tag).is_some_and(|w| w.wants_focus());
                if wants_focus {
                    self.deactivate_current(widgets, Some(&tag));
                    self.transition_to(Focus::Plugin(tag.into()));
                }
            }
            FocusEvent::PluginClosed(tag) => {
                if self.current.is_plugin(&tag) {
                    self.release();
                }
            }
        }
    }

    /// Cycle focus to the next (or previous) visible plugin, wrapping
    /// through Map. Tab is an explicit user intent, not a reactive
    /// transition, so it stays a dedicated method instead of riding
    /// `on`. Returns `true` if focus moved. Map → visible[0] → … →
    /// visible[last] → Map (reverse swaps the ends).
    pub fn cycle(&mut self, widgets: &mut PluginRegistry, forward: bool) -> bool {
        // Palette is modal and outside the plugin cycle; user must close
        // it first. (In practice palette.handle_key consumes Tab too, so
        // this branch is defensive.)
        if matches!(self.current, Focus::Palette) {
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
            // From Map, enter at the appropriate end of the list.
            Focus::Map => Some(if forward {
                visible.first().unwrap().clone()
            } else {
                visible.last().unwrap().clone()
            }),
            Focus::Palette => unreachable!("handled by early return above"),
            Focus::Plugin(cur) => {
                let cur_str = cur.as_ref();
                match visible.iter().position(|t| t == cur_str) {
                    Some(i) if forward => {
                        if i + 1 < visible.len() {
                            Some(visible[i + 1].clone())
                        } else {
                            None // past last → Map
                        }
                    }
                    Some(i) => {
                        if i > 0 {
                            Some(visible[i - 1].clone())
                        } else {
                            None // before first → Map
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
            Some(tag) => Focus::Plugin(tag.into()),
            None => Focus::Map,
        });
        true
    }

    // ── Internals ─────────────────────────────────────────────────

    /// Restore the remembered predecessor (or map if none).
    fn release(&mut self) {
        self.current = std::mem::replace(&mut self.prev, Focus::Map);
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
    /// Modal plugins close themselves through `deactivate`; non-modal
    /// plugins leave their panel visible. The policy lives in each
    /// plugin; this method just invokes it at the right moment.
    fn deactivate_current(&self, widgets: &mut PluginRegistry, keep_tag: Option<&str>) {
        if let Focus::Plugin(prev) = &self.current
            && keep_tag != Some(prev.as_ref())
            && let Some(p) = widgets.get_mut(prev.as_ref())
        {
            p.deactivate();
        }
    }
}
