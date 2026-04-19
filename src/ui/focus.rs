//! `Focus` — single source of truth for which widget (if any) has
//! exclusive keyboard focus. Mutated by widgets via `PluginCtx::focus`
//! (which still borrows `&mut Focus`), read by the dispatcher and layout
//! code. A thin `FocusManager` wraps the enum and owns the policy for
//! focus transitions that need to coordinate with the plugin registry
//! (deactivation callbacks in particular).

use std::borrow::Cow;

use crate::plugin::PluginRegistry;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    #[default]
    Map,
    Plugin(Cow<'static, str>),
}

impl Focus {
    pub fn is_plugin(&self, tag: &str) -> bool {
        matches!(self, Focus::Plugin(t) if t == tag)
    }
}

/// Coordinates focus transitions. The current `Focus` stays a public
/// field so plugins can still get a `&mut Focus` (via `&mut
/// focus.current`) and mutate it directly — only the multi-field
/// operations (like "release the outgoing plugin before moving on")
/// live here.
#[derive(Default)]
pub struct FocusManager {
    pub current: Focus,
}

impl FocusManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_plugin(&self, tag: &str) -> bool {
        self.current.is_plugin(tag)
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
        self.current = match next {
            Some(tag) => Focus::Plugin(tag.into()),
            None => Focus::Map,
        };
        true
    }
}
