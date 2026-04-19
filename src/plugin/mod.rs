//! UI widgets — self-contained components with their own state and rendering.
//!
//! Interactive widgets (search, help, wiki) implement the [`Plugin`]
//! trait so `keyboard.rs` can dispatch events to them uniformly without
//! hard-coding the per-widget `Action` mapping. Focus — which widget
//! currently owns the keyboard — is tracked on `UiState.focus` and
//! mutated through [`PluginCtx::focus`] from handler methods.

pub mod help;
pub mod here;
pub mod search;
pub mod wiki;

use crossterm::event::{KeyCode, KeyModifiers};
use indexmap::IndexMap;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::geo::LonLat;
use crate::keymap::{KeyBinding, parse_key_binding};
use crate::ui::focus::FocusManager;
use crate::ui::painter::MapPainter;
use crate::ui::theme::Theme;

/// Outcome of a widget seeing a raw key event.
#[derive(Debug, Clone, PartialEq)]
pub enum PluginAction {
    /// Key is not for this widget. Iteration should try the next widget
    /// and, if none claim it, the global keymap.
    Pass,
    /// Key consumed by the widget. App should redraw.
    Consumed,
    /// Plugin wants the map recentered on this location.
    Jump(LonLat),
}

/// Context passed to widget handler methods. Exposes shared app state
/// the widget may need to read (current map center) or mutate (focus).
/// Keeping this in a struct lets us grow the surface (e.g. a command
/// queue, a notification channel) without resignalling every widget.
pub struct PluginCtx<'a> {
    pub center: LonLat,
    pub focus: &'a mut FocusManager,
}

/// Interactive widget dispatched from the keyboard handler.
///
/// Widgets decide which keys and actions they consume; the keyboard
/// handler iterates them in priority order and never inspects
/// per-widget types. Focus is mutated through `ctx.focus` inside
/// handler methods.
pub trait Plugin {
    /// Stable identifier used by the registry and `Focus::Plugin`.
    /// Built-ins return a `&'static str`; plugins supply their own.
    fn tag(&self) -> &str;

    /// Short human-readable label used by the help overlay and other
    /// introspection surfaces. Empty means "opt out of help listing".
    fn description(&self) -> &str {
        ""
    }

    /// Key strings (parsed by `parse_key_binding`) that activate this
    /// widget. Registered at startup; the keyboard handler dispatches
    /// them to `activate` without going through the keymap.
    fn activation_keys(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Called when one of this widget's `activation_keys` is pressed.
    /// Default open/toggle/close semantics are the widget's own.
    fn activate(&mut self, _ctx: &mut PluginCtx<'_>) {}

    /// Called when focus moves to a different plugin via another
    /// plugin's activation key. Modal plugins (search, help) close
    /// themselves here; non-modal plugins leave their window visible
    /// and only release focus. Default is a no-op (non-modal).
    fn deactivate(&mut self) {}

    /// Whether this plugin's window is currently on screen. The main
    /// draw loop renders every plugin that reports `true`, regardless
    /// of focus — so non-modal panels (weather, status, wiki) can
    /// stay visible while the user is doing something else.
    fn visible(&self) -> bool {
        false
    }

    /// Raw key event while this widget holds focus. The handler is
    /// only called when the dispatcher routes to it — widgets do not
    /// need to self-gate. Return `Pass` only when the key is
    /// deliberately delegated back to the global keymap (e.g. the
    /// wiki panel, which passes non-nav keys through).
    fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        ctx: &mut PluginCtx<'_>,
    ) -> PluginAction;

    /// Drain any async/background work. Returns `true` if state
    /// changed and the app should redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Async jump request produced by the plugin (e.g. `here` resolves
    /// a geoip lookup started from the command palette). Called right
    /// after `poll`; returning `Some(loc)` makes the app recenter and
    /// redraw. Plugins that emit jumps only through `handle_key` keep
    /// the default.
    fn pending_jump(&mut self) -> Option<LonLat> {
        None
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
pub struct PluginRegistry {
    widgets: IndexMap<String, Box<dyn Plugin>>,
    activations: Vec<(KeyBinding, String)>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            widgets: IndexMap::new(),
            activations: Vec::new(),
        }
    }

    /// Register a widget. The tag comes from `Plugin::tag`; a
    /// duplicate tag replaces the prior entry. Each activation key
    /// the widget declares is recorded for later lookup.
    pub fn register(&mut self, w: Box<dyn Plugin>) {
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

    /// Move the plugin with this tag to the end of iteration order.
    /// Render / paint loops draw later entries on top, so the most
    /// recently activated plugin appears at the front.
    pub fn bring_to_front(&mut self, tag: &str) {
        if let Some(i) = self.widgets.get_index_of(tag) {
            let last = self.widgets.len().saturating_sub(1);
            if i < last {
                self.widgets.move_index(i, last);
            }
        }
    }

    pub fn get<'a>(&'a self, tag: &str) -> Option<&'a (dyn Plugin + 'a)> {
        self.widgets
            .get(tag)
            .map(|b| b.as_ref() as &(dyn Plugin + 'a))
    }

    pub fn get_mut<'a>(&'a mut self, tag: &str) -> Option<&'a mut (dyn Plugin + 'a)> {
        self.widgets
            .get_mut(tag)
            .map(|b| b.as_mut() as &mut (dyn Plugin + 'a))
    }

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = &'a (dyn Plugin + 'a)> + 'a {
        self.widgets
            .values()
            .map(|b| b.as_ref() as &(dyn Plugin + 'a))
    }

    pub fn iter_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut (dyn Plugin + 'a)> + 'a {
        self.widgets
            .values_mut()
            .map(|b| b.as_mut() as &mut (dyn Plugin + 'a))
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
