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

use std::time::Duration;

use crate::compositor::Context;
use crate::front::palette::action::PaletteAction;

/// One row in the palette list.
pub struct PaletteItem {
    pub label: String,
    pub hint: String,
}

/// When the palette should call [`PaletteProvider::filter`].
///
/// Static providers (commands, themes) match locally and can refilter
/// on every keystroke. Async providers (Nominatim, Wikipedia geosearch)
/// must avoid spamming the upstream API: either debounce on quiet,
/// or wait for the user to explicitly submit with Enter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitMode {
    /// Refilter on every query change. Default.
    OnEachKey,
    /// Refilter once `query` has been quiet for this duration.
    Debounced(Duration),
    /// Refilter only when the user explicitly submits — Enter on an
    /// empty list. Query mutations buffer silently. The provider
    /// stays in control of its async pipeline; the palette never
    /// fires `filter` from a timer.
    OnEnter,
}

/// Source of items + filter + activation logic for the palette popup.
pub trait PaletteProvider {
    /// Prompt string shown in front of the query (e.g. `":"` for the
    /// default provider, `"theme> "` for the theme provider).
    fn prompt(&self) -> &str;

    /// Rebuild the visible item list for this query. For async
    /// providers, this kicks off background work; results land later
    /// via [`Self::poll`].
    fn filter(&mut self, query: &str);

    /// Current visible items in display order.
    fn items(&self) -> &[PaletteItem];

    /// User pressed Enter on `items()[idx]`. `ctx` is forwarded so
    /// Spawn-kind entries can seed their component from app state.
    fn execute(&mut self, idx: usize, ctx: &Context) -> PaletteAction;

    /// User cancelled the palette — Esc, or Enter on an empty list.
    /// Default closes silently; providers override when they need to
    /// observe close (e.g. clean up a transient handle, log, etc.).
    /// Every close path that doesn't go through [`Self::execute`]
    /// funnels through here, so it doubles as the on-close hook.
    fn cancel(&mut self) -> PaletteAction {
        PaletteAction::Close
    }

    /// When the palette should invoke [`Self::filter`].
    fn submit_mode(&self) -> SubmitMode {
        SubmitMode::OnEachKey
    }

    /// Called every frame by the palette. Async providers use this to
    /// drain completion channels from background fetches and rebuild
    /// `items()`. Default no-op for sync providers.
    fn poll(&mut self) {}

    /// Whether the provider is waiting for an in-flight request. The
    /// palette renders a loading row while this is true.
    fn is_loading(&self) -> bool {
        false
    }
}
