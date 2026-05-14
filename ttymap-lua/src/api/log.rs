//! `ttymap.log` userdata — plugin-side logging sink.
//!
//! `target` is pre-formatted as `lua[<plugin>]` so callers don't pay
//! for the format on every line and `RUST_LOG=lua[aircraft]=debug`
//! filters cleanly. Mirrors the host-side
//! `log::warn!("lua[{tag}]: ...")` convention used elsewhere in the
//! bridge — same target shape, just opened up to scripts.

use mlua::UserData;

pub(super) struct HostLog {
    target: String,
}

impl HostLog {
    pub(super) fn new(target: String) -> Self {
        Self { target }
    }
}

impl UserData for HostLog {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("info", |_, this, msg: String| {
            log::info!(target: &this.target, "{}", msg);
            Ok(())
        });
        methods.add_method("warn", |_, this, msg: String| {
            log::warn!(target: &this.target, "{}", msg);
            Ok(())
        });
        methods.add_method("error", |_, this, msg: String| {
            log::error!(target: &this.target, "{}", msg);
            Ok(())
        });
    }
}
