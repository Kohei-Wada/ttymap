//! Palette provider abstraction.
//!
//! The palette popup is a **universal picker**: a generic UI that shows
//! a prompt, a filtered list, and lets the user navigate/select.
//! Providers plug in different item sources / filters / execution
//! behaviour (default command menu, theme switcher, ...).

pub mod command;
pub mod theme;

pub use command::{CommandProvider, CommandSeed};
pub use theme::ThemeProvider;

use crate::app::AppMsg;
use crate::compositor::{Component, Context};

/// One row in the palette list.
pub struct PaletteItem {
    pub label: String,
    pub hint: String,
}

/// What a provider wants the host to do when the user activates an
/// item. Translated by the palette Component into the equivalent
/// `win.*` calls.
pub enum PaletteAction {
    /// Close the palette with no side effect.
    Close,
    /// Close the palette and dispatch these messages.
    Run(Vec<AppMsg>),
    /// Close the palette and push `component` onto the compositor.
    /// Refocus semantic: if `component`'s concrete type is already
    /// on the stack, focus shifts to the existing instance.
    Push(Box<dyn Component>),
    /// Close the palette and toggle `component` onto the compositor.
    /// Close-on-collision semantic: if `component`'s concrete type is
    /// already on the stack, the existing instance is popped and
    /// `component` is dropped. Otherwise pushed like `Push`. Used by
    /// palette entries whose label promises toggle semantics.
    Toggle(Box<dyn Component>),
    /// Swap the palette's provider in place — sub-mode transition
    /// (e.g. "Theme"). Palette stays open.
    SwitchProvider(Box<dyn PaletteProvider>),
}

/// Source of items + filter + activation logic for the palette popup.
pub trait PaletteProvider {
    /// Prompt string shown in front of the query (e.g. `":"` for the
    /// default provider, `"theme> "` for the theme provider).
    fn prompt(&self) -> &str;

    /// Rebuild the visible item list for this query.
    fn filter(&mut self, query: &str);

    /// Current visible items in display order.
    fn items(&self) -> &[PaletteItem];

    /// User pressed Enter on `items()[idx]`. `ctx` is forwarded so
    /// Spawn-kind entries can seed their component from app state.
    fn execute(&mut self, idx: usize, ctx: &Context) -> PaletteAction;
}
