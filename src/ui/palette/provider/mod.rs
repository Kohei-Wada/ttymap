//! Palette provider abstraction.
//!
//! The palette popup is a **universal picker**: a generic UI that shows
//! a prompt, a filtered list, lets the user navigate and select. What
//! those items are, how they're filtered, and what happens on select
//! varies per use-case (run a command, switch theme, jump to a searched
//! location, pick a wiki article …).
//!
//! That variation lives behind [`PaletteProvider`]. The palette holds a
//! `Box<dyn PaletteProvider>` and delegates list / filter / execute to
//! it. Adding a new picker is implementing the trait in a sibling
//! module + teaching the palette how to reach it (typically by
//! returning [`PaletteAction::SwitchProvider`] from a parent provider).

pub mod command;
pub mod theme;

pub use command::CommandProvider;
pub use theme::ThemeProvider;

use crate::color_palette::ThemeId;
use crate::map::Action;

/// One row in the palette list.
pub struct PaletteItem {
    /// Main label — left-aligned, what the user reads.
    pub label: String,
    /// Right-side hint (keybind, metadata). Empty means no hint.
    pub hint: String,
}

/// What a provider wants the host to do when the user activates (Enter)
/// an item.
pub enum PaletteAction {
    /// Dismiss the palette.
    Close,
    /// Dispatch a map `Action` via `MapState::process_action`.
    Run(Action),
    /// Activate a plugin by tag (same as pressing its activation key).
    Activate(String),
    /// Switch the running theme.
    SetTheme(ThemeId),
    /// Swap to a different provider without closing the palette — the
    /// "sub-mode" transition. Query resets; focus stays.
    SwitchProvider(Box<dyn PaletteProvider>),
}

/// Source of items + filter + activation logic for the palette popup.
///
/// Providers are owned by the palette while visible. Instantiated when
/// the palette opens or switches mode; dropped when it closes.
pub trait PaletteProvider {
    /// Prompt string shown in front of the query (e.g. `":"` for the
    /// default command provider, `"theme> "` for the theme provider).
    fn prompt(&self) -> &str;

    /// Rebuild the visible item list for this query. Called on every
    /// query edit. Synchronous today; async providers (search, wiki)
    /// will need a polling extension when they arrive.
    fn filter(&mut self, query: &str);

    /// Current visible items in display order.
    fn items(&self) -> &[PaletteItem];

    /// User pressed Enter on `items()[idx]`. Returns what the host
    /// should do next.
    fn execute(&mut self, idx: usize) -> PaletteAction;
}
