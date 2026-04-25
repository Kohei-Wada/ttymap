//! Help plugin — keybinding cheatsheet shown as a centred popup.
//!
//! Under the compositor model: ephemeral component, fresh instance
//! on every push. Any key closes it.
//!
//! ## Layout
//!
//! - [`text`] — `HelpText`: pre-computed lines built once at startup
//! - [`component`] — `HelpComponent`: popup render + close-on-any-key

mod component;
mod text;

pub use text::HelpText;

use std::rc::Rc;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::plugin_api::prelude::*;

use component::HelpComponent;

/// Register the help plugin. Takes pre-computed help entries from
/// sibling plugins (harvested by the composition root) so help
/// remains in sync with what's actually loaded.
pub fn register(help_text: Rc<HelpText>, r: &mut Registrar) {
    let text_a = help_text.clone();
    r.bind(KeyCode::Char('?'), KeyModifiers::NONE, move |_| {
        HelpComponent::new(text_a.clone())
    });
    r.add_toggle("Toggle help", "?", move |_| {
        HelpComponent::new(help_text.clone())
    });
}
