//! `Focus` — single source of truth for which widget (if any) has
//! exclusive keyboard focus. Lives on `UiState`; mutated by widgets
//! via `WidgetCtx::focus`, read by the dispatcher to route keys and by
//! layout code to decide border colour / modal visibility.

use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Focus {
    #[default]
    Map,
    Widget(Cow<'static, str>),
}

impl Focus {
    pub fn is_widget(&self, tag: &str) -> bool {
        matches!(self, Focus::Widget(t) if t == tag)
    }
}
