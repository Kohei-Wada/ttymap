//! UI widgets — self-contained components with their own state and rendering.
//!
//! Interactive widgets (search, help, wiki) implement the [`Widget`]
//! trait so `keyboard.rs` can dispatch events to them uniformly without
//! hard-coding the per-widget `Action` mapping. Focus — which widget
//! currently owns the keyboard — is tracked on `UiState.focus` and
//! mutated through [`WidgetCtx::focus`] from handler methods.

pub mod help;
pub mod map;
pub mod overlay;
pub mod search;
pub mod wiki;

use crossterm::event::{KeyCode, KeyModifiers};
use indexmap::IndexMap;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::geo::LonLat;
use crate::keymap::{KeyBinding, parse_key_binding};
use crate::ui::focus::Focus;
use crate::ui::painter::MapPainter;
use crate::ui::theme::Theme;

/// Outcome of a widget seeing a raw key event.
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetAction {
    /// Key is not for this widget. Iteration should try the next widget
    /// and, if none claim it, the global keymap.
    Pass,
    /// Key consumed by the widget. App should redraw.
    Consumed,
    /// Widget wants the map recentered on this location.
    Jump(LonLat),
}

/// Context passed to widget handler methods. Exposes shared app state
/// the widget may need to read (current map center) or mutate (focus).
/// Keeping this in a struct lets us grow the surface (e.g. a command
/// queue, a notification channel) without resignalling every widget.
pub struct WidgetCtx<'a> {
    pub center: LonLat,
    pub focus: &'a mut Focus,
}

/// Interactive widget dispatched from the keyboard handler.
///
/// Widgets decide which keys and actions they consume; the keyboard
/// handler iterates them in priority order and never inspects
/// per-widget types. Focus is mutated through `ctx.focus` inside
/// handler methods.
pub trait Widget {
    /// Stable identifier used by the registry and `Focus::Widget`.
    /// Built-ins return a `&'static str`; plugins supply their own.
    fn tag(&self) -> &str;

    /// Key strings (parsed by `parse_key_binding`) that activate this
    /// widget. Registered at startup; the keyboard handler dispatches
    /// them to `activate` without going through the keymap.
    fn activation_keys(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Called when one of this widget's `activation_keys` is pressed.
    /// Default open/toggle/close semantics are the widget's own.
    fn activate(&mut self, _ctx: &mut WidgetCtx<'_>) {}

    /// Called when focus moves to a different widget via another
    /// widget's activation key. Widgets should close / clear any
    /// state that shouldn't outlive losing focus (e.g. wiki's article
    /// list is kept, but its "panel is open" flag is cleared so
    /// markers stop rendering).
    fn deactivate(&mut self) {}

    /// Raw key event while this widget holds focus. The handler is
    /// only called when the dispatcher routes to it — widgets do not
    /// need to self-gate. Return `Pass` only when the key is
    /// deliberately delegated back to the global keymap (e.g. the
    /// wiki panel, which passes non-nav keys through).
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut WidgetCtx<'_>,
    ) -> WidgetAction;

    /// Drain any async/background work. Returns `true` if state
    /// changed and the app should redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Render the widget's modal panel. Called only when the widget
    /// holds focus; widgets that don't have a panel leave this as a
    /// no-op.
    fn render(&self, _f: &mut Frame, _area: Rect, _theme: &Theme) {}

    /// Context-sensitive key hints for the footer, shown while the
    /// widget holds focus.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Paint primitives on the map via the supplied `MapPainter`.
    /// Always called during the draw phase, regardless of focus — a
    /// widget can leave its markers on the map even while another
    /// widget holds the keyboard.
    fn paint_on_map(&self, _p: &mut MapPainter<'_>) {}
}

/// Ordered registry of interactive widgets. Built-ins register at app
/// startup; plugins register as they're loaded. Activation-key
/// bindings declared by each widget are indexed here so the keyboard
/// handler can look them up without knowing any widget name.
pub struct WidgetRegistry {
    widgets: IndexMap<String, Box<dyn Widget>>,
    activations: Vec<(KeyBinding, String)>,
}

impl WidgetRegistry {
    pub fn new() -> Self {
        Self {
            widgets: IndexMap::new(),
            activations: Vec::new(),
        }
    }

    /// Register a widget. The tag comes from `Widget::tag`; a
    /// duplicate tag replaces the prior entry. Each activation key
    /// the widget declares is recorded for later lookup.
    pub fn register(&mut self, w: Box<dyn Widget>) {
        let tag = w.tag().to_string();
        for key_str in w.activation_keys() {
            match parse_key_binding(key_str) {
                Some(binding) => self.activations.push((binding, tag.clone())),
                None => log::warn!("invalid activation key {:?} for widget {:?}", key_str, tag),
            }
        }
        self.widgets.insert(tag, w);
    }

    /// Return the tag of the widget that claims this key, if any.
    pub fn activation_tag(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&str> {
        let clean_mods = modifiers & !KeyModifiers::SHIFT;
        self.activations
            .iter()
            .find(|(b, _)| b.code == code && b.modifiers == clean_mods)
            .map(|(_, tag)| tag.as_str())
    }

    pub fn get<'a>(&'a self, tag: &str) -> Option<&'a (dyn Widget + 'a)> {
        self.widgets
            .get(tag)
            .map(|b| b.as_ref() as &(dyn Widget + 'a))
    }

    pub fn get_mut<'a>(&'a mut self, tag: &str) -> Option<&'a mut (dyn Widget + 'a)> {
        self.widgets
            .get_mut(tag)
            .map(|b| b.as_mut() as &mut (dyn Widget + 'a))
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = &'a (dyn Widget + 'a)> + 'a {
        self.widgets
            .values()
            .map(|b| b.as_ref() as &(dyn Widget + 'a))
    }

    pub fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut (dyn Widget + 'a)> + 'a {
        self.widgets
            .values_mut()
            .map(|b| b.as_mut() as &mut (dyn Widget + 'a))
    }
}

impl Default for WidgetRegistry {
    fn default() -> Self {
        Self::new()
    }
}
