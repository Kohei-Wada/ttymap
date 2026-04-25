//! Scalebar plugin — bottom-right always-on overlay showing a
//! distance scale derived from the current zoom + centre latitude.
//!
//! Replaces the legacy `ScaleBarOverlay`. Stateless: every frame
//! recomputes the bar from the rendered map snapshot via
//! [`crate::geo::scale_bar`].

mod component;

use crate::plugin_api::prelude::*;

use component::ScalebarComponent;

pub fn register(r: &mut Registrar) {
    r.add_overlay(|_ctx| ScalebarComponent);
}
