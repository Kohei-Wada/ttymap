//! `ttymap.config` userdata — read-only access to host-side config
//! values plugins may need at runtime.

use std::sync::Arc;

use mlua::UserData;

use super::LuaHostShared;

pub(super) struct HostConfig {
    shared: Arc<LuaHostShared>,
}

impl HostConfig {
    pub(super) fn new(shared: Arc<LuaHostShared>) -> Self {
        Self { shared }
    }
}

impl UserData for HostConfig {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.config:geoip_endpoint() -> string` — configured geoip
        // URL (`ttymap.opt.geoip.endpoint` in init.lua). The here
        // plugin GETs this to resolve the user's location.
        methods.add_method("geoip_endpoint", |_, this, _: ()| {
            Ok(this.shared.geoip_endpoint.clone())
        });
    }
}
