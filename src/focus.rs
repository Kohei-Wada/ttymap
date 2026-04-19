//! `Focus` — single source of truth for which widget (if any) has
//! exclusive keyboard focus. Read by the dispatcher and layout code,
//! mutated by widgets via `PluginCtx::focus` (which still borrows
//! `&mut Focus`). `FocusManager` is the single gatekeeper: the inner
//! `Focus` is private, exposed for reads via `current()` and for
//! plugin-context construction via `plugin_slot()`. Routing code (key
//! dispatch, Tab cycling) must go through `cycle` / `deactivate_focused`
//! so deactivation callbacks can't be bypassed by accident.

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

/// Coordinates focus transitions. The inner `Focus` is private so
/// every transition runs through a method that also updates the
/// `prev` slot — that's how `release()` restores the focus a plugin
/// held before the current one grabbed it, instead of always dumping
/// back to the map.
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

    /// Plugin-facing: take focus for this tag. The previous focus (map
    /// or another plugin) is remembered so `release()` can restore it.
    pub fn take(&mut self, tag: impl Into<std::borrow::Cow<'static, str>>) {
        self.transition_to(Focus::Plugin(tag.into()));
    }

    /// Builtin palette-facing: take focus for the command palette.
    pub fn take_palette(&mut self) {
        self.transition_to(Focus::Palette);
    }

    /// Plugin-facing: release focus, returning it to whoever held it
    /// before this plugin took over. Falls back to the map if there's
    /// no remembered predecessor.
    pub fn release(&mut self) {
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

    /// Call `deactivate` on the currently-focused plugin unless the
    /// caller is about to re-activate the same one (toggle case).
    /// Modal plugins close themselves through `deactivate`; non-modal
    /// plugins leave their panel visible. The policy lives in each
    /// plugin, this method just invokes it at the right moment.
    pub fn deactivate_focused(&self, widgets: &mut PluginRegistry, keep_tag: Option<&str>) {
        if let Focus::Plugin(prev) = &self.current
            && keep_tag != Some(prev.as_ref())
            && let Some(p) = widgets.get_mut(prev.as_ref())
        {
            p.deactivate();
        }
    }

    /// Cycle focus to the next (or previous) visible plugin, wrapping
    /// through Map. Returns `true` if focus moved. Map → visible[0] →
    /// … → visible[last] → Map (reverse swaps the ends).
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

        self.deactivate_focused(widgets, None);
        self.transition_to(match next {
            Some(tag) => Focus::Plugin(tag.into()),
            None => Focus::Map,
        });
        true
    }
}
