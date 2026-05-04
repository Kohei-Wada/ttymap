//! `ttymap.tile` userdata — read-only access to the active tile
//! provider's metadata.

use std::sync::Arc;

use mlua::UserData;

use crate::lua::host::LuaHostShared;

pub(super) struct HostTile {
    shared: Arc<LuaHostShared>,
}

impl HostTile {
    pub(super) fn new(shared: Arc<LuaHostShared>) -> Self {
        Self { shared }
    }
}

impl UserData for HostTile {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.tile:attribution() -> string | nil` — active
        // TileClient's attribution string (typically "© OpenStreetMap
        // …"). The attribution overlay paints this; other plugins may
        // use it for their own attribution rows.
        methods.add_method("attribution", |_, this, _: ()| {
            Ok(this.shared.attribution.clone())
        });
    }
}
