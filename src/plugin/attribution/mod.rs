//! Attribution plugin — bottom-left always-on overlay rendering the
//! tile provider's attribution string. Replaces the legacy
//! `AttributionOverlay`.
//!
//! Skipped silently when no attribution text was configured (the
//! `Option<String>` from `build_tile_cache`).

mod component;

use crate::plugin_api::prelude::*;

use component::AttributionComponent;

pub fn register(text: Option<String>, r: &mut Registrar) {
    let Some(text) = text.filter(|s| !s.is_empty()) else {
        return;
    };
    r.add_overlay(move |_ctx| AttributionComponent::new(text.clone()));
}
