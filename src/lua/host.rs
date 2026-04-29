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
//! - `host:parse_json(s) -> value | nil` — turn a JSON string into
//!   nested Lua tables. Objects become string-keyed tables, arrays
//!   become 1-indexed tables, `null` is `nil`. Parse errors return
//!   `nil` and log a warning, so a flaky upstream doesn't crash a
//!   plugin.
//! - `host:center() -> lon, lat` — current map centre. The host
//!   refreshes the value at the start of every dispatch path that
//!   carries a `Window` / `MapApi`, so callbacks see the latest
//!   centre without threading anything through their signatures.
//!
//! Both [`LuaHost`] and [`LuaJob`] are `'static`, so they go through
//! mlua's regular `UserData` mechanism (no `Lua::scope` gymnastics
//! like `MapApi`).

use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use mlua::UserData;

use crate::geo::LonLat;
use crate::shared::http::HttpClient;

/// Per-component host handle. Owns the `HttpClient` used by
/// `fetch_url` and a sender for jump requests. The matching
/// `Receiver` lives on the `LuaComponent` so it can drain pending
/// jumps after a callback returns. The `center` is shared with
/// the component so `host:center()` can return the current map
/// centre even though the userdata itself never sees a `Window`.
pub struct LuaHost {
    http: HttpClient,
    jump_tx: mpsc::Sender<LonLat>,
    center: Arc<Mutex<LonLat>>,
}

impl LuaHost {
    /// Build a fresh host along with the channel ends the
    /// [`LuaComponent`] needs to drive it: the jump-request
    /// receiver and the shared centre cell. The component refreshes
    /// the centre at the start of each dispatch path that carries
    /// a `Window` / `MapApi`.
    pub fn new(tag: &'static str) -> (Self, mpsc::Receiver<LonLat>, Arc<Mutex<LonLat>>) {
        let (jump_tx, jump_rx) = mpsc::channel();
        let center = Arc::new(Mutex::new(LonLat { lon: 0.0, lat: 0.0 }));
        (
            Self {
                http: HttpClient::new(tag),
                jump_tx,
                center: center.clone(),
            },
            jump_rx,
            center,
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

        // `host:parse_json(s) -> value | nil` — JSON → Lua. Errors
        // (invalid input, non-finite numbers, etc.) become `nil`
        // with a warning log; the caller can fall back to a default
        // without a stack trace.
        methods.add_method(
            "parse_json",
            |lua, _this, source: String| match serde_json::from_str::<serde_json::Value>(&source) {
                Ok(v) => json_to_lua(lua, &v).map(Some),
                Err(e) => {
                    log::warn!("lua-host: parse_json failed: {}", e);
                    Ok(None)
                }
            },
        );

        // `host:center() -> lon, lat` — current map centre, kept
        // fresh by the LuaComponent before each dispatch. Plugins
        // use this to scope upstream queries (e.g. an OpenSky
        // bounding box around the user's view).
        methods.add_method("center", |_, this, _: ()| {
            let ll = *this.center.lock().expect("center mutex poisoned");
            Ok((ll.lon, ll.lat))
        });
    }
}

/// Recursive translation of a `serde_json::Value` into a
/// `mlua::Value`. Objects map to string-keyed tables, arrays to
/// 1-indexed tables (Lua convention), null to nil, integers to
/// `Integer` when they fit and `Number` otherwise.
fn json_to_lua(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    match value {
        serde_json::Value::Null => Ok(mlua::Value::Nil),
        serde_json::Value::Bool(b) => Ok(mlua::Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(mlua::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(mlua::Value::Number(f))
            } else {
                // Numbers that fit neither i64 nor f64 are
                // exotic (large unsigned). Surface as nil rather
                // than panic; plugins can do their own handling.
                Ok(mlua::Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(mlua::Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(items) => {
            let table = lua.create_table()?;
            // Lua arrays are 1-indexed.
            for (i, item) in items.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, item)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(mlua::Value::Table(table))
        }
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
        let (host, _jump_rx, _) = LuaHost::new("lua-test");
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
        let (host, jump_rx, _) = LuaHost::new("lua-test");
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
    fn parse_json_round_trips_primitives() {
        let lua = mlua::Lua::new();
        let (host, _, _) = LuaHost::new("lua-test");
        lua.globals()
            .set("host", lua.create_userdata(host).unwrap())
            .unwrap();
        let n: i64 = lua
            .load(r#"return host:parse_json("42")"#)
            .eval()
            .expect("eval");
        assert_eq!(n, 42);
        let s: String = lua
            .load(r#"return host:parse_json('"hi"')"#)
            .eval()
            .expect("eval");
        assert_eq!(s, "hi");
        let b: bool = lua
            .load(r#"return host:parse_json("true")"#)
            .eval()
            .expect("eval");
        assert!(b);
    }

    #[test]
    fn parse_json_object_becomes_string_keyed_table() {
        let lua = mlua::Lua::new();
        let (host, _, _) = LuaHost::new("lua-test");
        lua.globals()
            .set("host", lua.create_userdata(host).unwrap())
            .unwrap();
        let (name, age): (String, i64) = lua
            .load(
                r#"
                local t = host:parse_json('{"name": "alice", "age": 30}')
                return t.name, t.age
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(name, "alice");
        assert_eq!(age, 30);
    }

    #[test]
    fn parse_json_array_is_one_indexed_in_lua() {
        let lua = mlua::Lua::new();
        let (host, _, _) = LuaHost::new("lua-test");
        lua.globals()
            .set("host", lua.create_userdata(host).unwrap())
            .unwrap();
        // Lua arrays are 1-indexed; t[1] is the first element.
        let (first, third, len): (i64, i64, i64) = lua
            .load(
                r#"
                local t = host:parse_json("[10, 20, 30]")
                return t[1], t[3], #t
                "#,
            )
            .eval()
            .expect("eval");
        assert_eq!(first, 10);
        assert_eq!(third, 30);
        assert_eq!(len, 3);
    }

    #[test]
    fn parse_json_invalid_returns_nil() {
        let lua = mlua::Lua::new();
        let (host, _, _) = LuaHost::new("lua-test");
        lua.globals()
            .set("host", lua.create_userdata(host).unwrap())
            .unwrap();
        let v: mlua::Value = lua
            .load(r#"return host:parse_json("not json !")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
    }

    #[test]
    fn parse_json_null_is_nil() {
        let lua = mlua::Lua::new();
        let (host, _, _) = LuaHost::new("lua-test");
        lua.globals()
            .set("host", lua.create_userdata(host).unwrap())
            .unwrap();
        let v: mlua::Value = lua
            .load(r#"return host:parse_json("null")"#)
            .eval()
            .expect("eval");
        assert!(matches!(v, mlua::Value::Nil), "got {:?}", v);
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
