//! Lua-side host services: persistent state a Lua plugin can reach
//! at any time via the `host` global.
//!
//! Today:
//! - `host:fetch_url(url) -> Job` — kicks off a background HTTP GET
//!   (UTF-8 text). The plugin polls the [`LuaJob`] each frame.
//! - `host:jump(lon, lat)` — fire-and-forget request to recentre
//!   the map on the given coordinate. The Lua side just enqueues a
//!   `LonLat`; [`LuaComponent`] drains the channel after each
//!   `poll` / `handle_event` dispatch and emits `AppMsg::Jump`
//!   through the host `Window`. This keeps the Lua call site
//!   independent of when a `Window` is actually available.
//!
//! Both [`LuaHost`] and [`LuaJob`] are `'static`, so they go through
//! mlua's regular `UserData` mechanism (no `Lua::scope` gymnastics
//! like `MapApi`).

use std::sync::mpsc;
use std::thread;

use mlua::UserData;

use crate::geo::LonLat;
use crate::shared::http::HttpClient;

/// Per-component host handle. Owns the `HttpClient` used by
/// `fetch_url` and a sender for jump requests. The matching
/// `Receiver` lives on the `LuaComponent` so it can drain pending
/// jumps after a callback returns.
pub struct LuaHost {
    http: HttpClient,
    jump_tx: mpsc::Sender<LonLat>,
}

impl LuaHost {
    /// Build a fresh host + the matching jump-channel receiver.
    /// The receiver belongs on the [`LuaComponent`] that owns this
    /// `LuaHost`'s Lua state.
    pub fn new(tag: &'static str) -> (Self, mpsc::Receiver<LonLat>) {
        let (jump_tx, jump_rx) = mpsc::channel();
        (
            Self {
                http: HttpClient::new(tag),
                jump_tx,
            },
            jump_rx,
        )
    }
}

impl UserData for LuaHost {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `host:fetch_url(url)` — spawn a background GET and return a
        // Job. Body is decoded as UTF-8; non-text or fetch errors
        // surface as the Job never producing a result (try_take
        // keeps returning nil).
        // `add_method` auto-extracts `self` from Lua's colon
        // syntax, so the closure args are just the actual user
        // params — `(url)` here.
        methods.add_method("fetch_url", |_, this, url: String| {
            Ok(LuaJob::spawn(&this.http, url))
        });

        // `host:jump(lon, lat)` — request the map recentre on the
        // given coordinate. The actual `AppMsg::Jump` emit happens
        // when the matching `LuaComponent` drains the channel after
        // its current callback returns, so this is fire-and-forget
        // from the Lua side. Send errors (channel disconnected)
        // mean the component is being torn down — silently ignore.
        methods.add_method("jump", |_, this, (lon, lat): (f64, f64)| {
            let _ = this.jump_tx.send(LonLat { lon, lat });
            Ok(())
        });
    }
}

/// One-shot fetch handle. Stays alive in the Lua state until the
/// plugin drops its reference (or until the Lua state itself is
/// dropped, which happens when the LuaComponent is rebuilt).
pub struct LuaJob {
    rx: mpsc::Receiver<String>,
}

impl LuaJob {
    fn spawn(http: &HttpClient, url: String) -> Self {
        let (tx, rx) = mpsc::channel();
        let http = http.clone();
        thread::spawn(move || {
            // Errors are silent for now: a Lua plugin that needs
            // error visibility can poll a deadline of its own. We
            // log so offline debugging has a hook.
            match http.get_bytes(&url) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(body) => {
                        let _ = tx.send(body);
                    }
                    Err(e) => log::warn!("lua-host: fetch_url {}: not utf-8: {}", url, e),
                },
                Err(e) => log::warn!("lua-host: fetch_url {}: {}", url, e),
            }
        });
        Self { rx }
    }
}

impl UserData for LuaJob {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `job:try_take() -> string | nil` — non-blocking. Returns
        // the body once it arrives, or nil while the fetch is
        // still in flight (or has failed).
        methods.add_method_mut("try_take", |_, this, _: mlua::Variadic<mlua::Value>| {
            Ok(this.rx.try_recv().ok())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_userdata_can_be_constructed() {
        let lua = mlua::Lua::new();
        let (host, _jump_rx) = LuaHost::new("lua-test");
        let ud = lua.create_userdata(host).expect("create_userdata");
        // Just confirm we can set it as a global and round-trip the
        // userdata reference back out — fetch_url itself hits the
        // network and isn't safe to call in a unit test.
        lua.globals().set("host", ud).expect("set global");
        let _: mlua::AnyUserData = lua.globals().get("host").expect("get global");
    }

    #[test]
    fn host_jump_pushes_to_channel() {
        let lua = mlua::Lua::new();
        let (host, jump_rx) = LuaHost::new("lua-test");
        let ud = lua.create_userdata(host).expect("create_userdata");
        lua.globals().set("host", ud).expect("set global");

        // Lua-side call: longitude first, then latitude.
        lua.load("host:jump(139.7595, 35.6828)")
            .exec()
            .expect("exec");

        let ll = jump_rx.try_recv().expect("jump must be queued");
        assert!((ll.lon - 139.7595).abs() < 1e-9);
        assert!((ll.lat - 35.6828).abs() < 1e-9);
    }

    #[test]
    fn job_try_take_returns_nil_before_send() {
        // Build a job by hand (skip the HTTP path) so we can
        // assert try_take's non-blocking behaviour.
        let (tx, rx) = mpsc::channel::<String>();
        let job = LuaJob { rx };
        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(job).expect("create_userdata");
        let result: Option<String> = lua
            .load("return select(1, ...):try_take()")
            .call(ud.clone())
            .expect("call");
        assert!(result.is_none(), "try_take should be nil before send");

        // Send a value and the next try_take returns it.
        tx.send("hi".to_string()).unwrap();
        let result: Option<String> = lua
            .load("return select(1, ...):try_take()")
            .call(ud)
            .expect("call");
        assert_eq!(result.as_deref(), Some("hi"));
    }
}
