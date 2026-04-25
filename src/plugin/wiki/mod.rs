//! Wiki plugin — Wikipedia geosearch panel, map markers, and the
//! background fetcher that populates them.
//!
//! Under the compositor model wiki is a single
//! [`component::WikiComponent`] with two rendering hooks:
//!
//! - `render` — the side panel (list / detail view), modal popup
//! - `paint_on_map` — article markers drawn on the map
//!
//! Both hooks fire only while the component is on the compositor
//! stack, so opening the panel shows the markers *and* the list, and
//! closing the panel removes both in step — no separate "paint
//! active?" flag to keep in sync.
//!
//! The persistent article list, selection, and detail state live in
//! [`state::WikiState`] behind an `Rc<RefCell<_>>` so a future second
//! push of the wiki panel (from the palette, say) sees the same
//! cached list without a re-fetch. The handle is owned by the spawn
//! closures in `register` and lives for the app's lifetime.
//!
//! ## Layout
//!
//! - [`state`] — `WikiState`: feed + cached articles + selection + detail
//! - [`component`] — `WikiComponent`: list/detail key handling + markers
//! - [`panel`] — side-panel render (consumed by `component::render`)
//! - [`wikipedia`] — HTTP client + JSON parsers (private)

mod component;
pub mod panel;
mod state;
mod wikipedia;

pub use state::WikiState;

use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyModifiers};
use serde::Deserialize;

use crate::config::Config;
use crate::plugin_api::prelude::*;

use component::WikiComponent;
use state::WikiHandle;

/// Wiki plugin config (`[wiki]` in config.toml).
///
/// `language` defaults to `"en"` so users running the plugin
/// out-of-box hit english Wikipedia; set this independently of the
/// renderer's `[render].language` if the user wants e.g. Japanese
/// map labels but English articles.
#[derive(Deserialize)]
#[serde(default)]
pub struct WikiConfig {
    pub language: String,
    pub limit: u32,
}

impl Default for WikiConfig {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            limit: 50,
        }
    }
}

/// Wire wiki into the registrar. Creates the shared state once; the
/// spawn closures for activation (`i`) and the palette entry both
/// clone the handle so every push shares the same persistent list.
pub fn register(config: &Config, r: &mut Registrar) {
    let cfg: WikiConfig = config.plugin("wiki");
    let state: WikiHandle = Rc::new(RefCell::new(WikiState::new(&cfg.language, cfg.limit)));
    let state_for_key = state.clone();
    r.bind(KeyCode::Char('i'), KeyModifiers::NONE, move |ctx| {
        WikiComponent::new(state_for_key.clone(), ctx.center)
    });
    r.add_toggle("Toggle wiki", "i", move |ctx| {
        WikiComponent::new(state.clone(), ctx.center)
    });
}
