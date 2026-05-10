//! `ttymap.config` userdata — read-only access to host-side config
//! values plugins may need at runtime.
//!
//! Currently empty: every former accessor (`:geoip_endpoint()`) was
//! a third-party-API-specific endpoint that belonged in Lua-side
//! plugin config (`runtime/lua/ttymap/<name>.lua`), not on the host
//! struct. Kept as a userdata stub so `ttymap.config` still
//! resolves; future plugin-agnostic accessors land here.

use std::sync::Arc;

use mlua::UserData;

use crate::lua::host::LuaHostShared;

pub(super) struct HostConfig {
    #[allow(dead_code)]
    shared: Arc<LuaHostShared>,
}

impl HostConfig {
    pub(super) fn new(shared: Arc<LuaHostShared>) -> Self {
        Self { shared }
    }
}

impl UserData for HostConfig {}
