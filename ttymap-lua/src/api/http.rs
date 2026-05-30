//! `ttymap.http` — HTTP fetch surface for Lua plugins.
//!
//! Two methods:
//!
//! - `:fetch(url [, opts])` — background request, returns a `LuaJob`
//!   userdata the plugin polls with `:try_take()`. Body is decoded as
//!   UTF-8; non-text or fetch errors surface as the Job never
//!   producing a result. `opts` (a table) unlocks auth'd / non-GET
//!   requests without endpoint-specific Rust:
//!   - `method` — `"POST"` (default `"GET"`)
//!   - `headers` — `{ ["Authorization"] = "Bearer …" }`
//!   - `form` — `{ k = v }` sent as `x-www-form-urlencoded` (OAuth2
//!     client-credentials token exchange, …)
//! - `:fetch_cached(url, ttl_secs)` — read-through disk cache. On
//!   fresh-enough hit (`age < ttl_secs`) emits the cached body
//!   without touching the network. On miss, real fetch + write-
//!   through. On HTTP error, falls back to the stale on-disk copy
//!   if one exists — critical for upstreams (e.g. CelesTrak's
//!   `gp.php`) that 403 a same-IP repeat fetch within their own
//!   refresh window.
//! - `:url_encode(s)` — percent-encode a query string per RFC 3986.
//!
//! `LuaJob` is the matching one-shot fetch handle. It stays alive
//! in the Lua state until the plugin drops its reference (or until
//! the setup-state Lua VM itself is dropped at program exit).
//! Lua-side: `job:try_take()` polls for the body, `job:cancel()`
//! requests early disposal — the worker drops its result, and any
//! already-buffered body becomes unreachable from `try_take`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::SystemTime;

use mlua::UserData;

use ttymap_engine::shared::http::HttpClient;

pub struct HostHttp {
    pub http: HttpClient,
    /// Resolved cache root passed in from `ttymap-config::AppDirs`
    /// (#362). The `lua-http/` subdir is appended at write time.
    /// `None` disables on-disk caching for `fetch_cached`.
    pub cache_root: Option<std::path::PathBuf>,
}

/// A Lua-specified request: method + extra headers + optional form
/// body. `Default` is a plain GET, so the bare `fetch(url)` and the
/// cached path build one without parsing any options.
#[derive(Default)]
struct RequestSpec {
    method: String,
    headers: Vec<(String, String)>,
    form: Vec<(String, String)>,
}

/// Flatten a `{ name = value }` Lua table into ordered pairs. Missing
/// table → empty; non-string entries are skipped silently.
fn table_to_pairs(t: Option<mlua::Table>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(t) = t {
        for entry in t.pairs::<String, String>().flatten() {
            out.push(entry);
        }
    }
    out
}

fn parse_request_spec(opts: Option<mlua::Table>) -> mlua::Result<RequestSpec> {
    let Some(opts) = opts else {
        return Ok(RequestSpec::default());
    };
    let method = opts
        .get::<Option<String>>("method")?
        .unwrap_or_else(|| "GET".to_string());
    // `request_bytes` is GET-or-POST only; reject anything else at the
    // Lua boundary so a typo (`"DELTE"`) errors instead of silently
    // falling through to a GET.
    if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("POST") {
        return Err(mlua::Error::external(format!(
            "ttymap.http:fetch: unsupported method {method:?} (expected \"GET\" or \"POST\")"
        )));
    }
    Ok(RequestSpec {
        method,
        headers: table_to_pairs(opts.get::<Option<mlua::Table>>("headers")?),
        form: table_to_pairs(opts.get::<Option<mlua::Table>>("form")?),
    })
}

impl UserData for HostHttp {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "fetch",
            |_, this, (url, opts): (String, Option<mlua::Table>)| {
                let spec = parse_request_spec(opts)?;
                Ok(LuaJob::spawn(
                    &this.http,
                    url,
                    spec,
                    None,
                    this.cache_root.clone(),
                ))
            },
        );
        methods.add_method("fetch_cached", |_, this, (url, ttl_secs): (String, u64)| {
            Ok(LuaJob::spawn(
                &this.http,
                url,
                RequestSpec::default(),
                Some(ttl_secs),
                this.cache_root.clone(),
            ))
        });
        methods.add_method("url_encode", |_, _this, s: String| {
            Ok(ttymap_engine::shared::http::url::urlencoded(&s))
        });
    }
}

/// One-shot fetch handle. Stays alive in the Lua state until the
/// plugin drops its reference (or until the setup-state Lua VM
/// itself is dropped at program exit).
///
/// `cancelled` is a flag the spawning thread checks at two points
/// (before the cache hit send and before the post-fetch send) and
/// `try_take` honours after the fact. Calling `cancel()` makes the
/// job poll-empty from then on regardless of whether the worker
/// finished — the in-flight HTTP request itself isn't aborted (the
/// underlying `reqwest` GET runs to completion, just into the void),
/// so the only cost of a cancelled fetch is the worker thread
/// finishing its current iteration silently.
pub struct LuaJob {
    rx: mpsc::Receiver<String>,
    cancelled: Arc<AtomicBool>,
}

impl LuaJob {
    /// Background HTTP GET. With `cache_ttl == None` it's a plain
    /// fetch; with `Some(ttl_secs)` it's read-through against a
    /// disk cache (write-through on success, stale-fallback on HTTP
    /// error so a rate-limiting upstream doesn't strand callers on
    /// "no body, no error"). Cache miss / stale rolls into the
    /// network path automatically.
    fn spawn(
        http: &HttpClient,
        url: String,
        spec: RequestSpec,
        cache_ttl: Option<u64>,
        cache_root: Option<std::path::PathBuf>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_worker = cancelled.clone();
        let http = http.clone();
        let path = cache_ttl.and_then(|_| http_cache_path(cache_root.as_deref(), &url));
        thread::spawn(move || {
            // Fresh cache hit → return immediately, skip the network.
            if let (Some(ttl), Some(p)) = (cache_ttl, path.as_ref())
                && let Ok(meta) = std::fs::metadata(p)
                && let Ok(modified) = meta.modified()
                && let Ok(age) = SystemTime::now().duration_since(modified)
                && age.as_secs() < ttl
                && let Ok(body) = std::fs::read_to_string(p)
            {
                if !cancelled_for_worker.load(Ordering::Relaxed) {
                    let _ = tx.send(body);
                }
                return;
            }

            // Cache miss / stale → real fetch.
            match http.request_bytes(&spec.method, &url, &spec.headers, &spec.form) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(body) => {
                        if let Some(p) = path.as_ref() {
                            if let Some(parent) = p.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(p, &body);
                        }
                        if !cancelled_for_worker.load(Ordering::Relaxed) {
                            let _ = tx.send(body);
                        }
                    }
                    Err(e) => log::warn!("lua-host: http:fetch {}: not utf-8: {}", url, e),
                },
                Err(e) => {
                    log::warn!("lua-host: http:fetch {}: {}", url, e);
                    // Cached caller? Try to serve a stale copy so a
                    // flaky upstream doesn't strand them.
                    if let Some(p) = path.as_ref()
                        && let Ok(body) = std::fs::read_to_string(p)
                        && !cancelled_for_worker.load(Ordering::Relaxed)
                    {
                        log::info!("lua-host: http:fetch {}: using stale cache", url);
                        let _ = tx.send(body);
                    }
                }
            }
        });
        Self { rx, cancelled }
    }
}

impl UserData for LuaJob {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("try_take", |_, this, _: mlua::Variadic<mlua::Value>| {
            // Cancelled jobs report no body even if the worker
            // raced and managed to send one before the cancel
            // landed — `cancel()` should look the same to the
            // caller regardless of timing.
            if this.cancelled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            Ok(this.rx.try_recv().ok())
        });
        // Idempotent — calling cancel twice (or after the worker
        // has already produced a body) is a harmless no-op.
        methods.add_method("cancel", |_, this, _: ()| {
            this.cancelled.store(true, Ordering::Relaxed);
            Ok(())
        });
    }
}

/// Where we stash an HTTP response for `fetch_cached`. FNV-1a 64-bit
/// keeps the cache key deterministic across runs (Rust's default
/// hasher is not), and small enough to fit on every filesystem.
/// `cache_root` comes from `ttymap-config::AppDirs.cache` (#362);
/// `None` means the caller couldn't resolve a per-user cache dir
/// and treats every read as a permanent miss + no write.
fn http_cache_path(cache_root: Option<&std::path::Path>, url: &str) -> Option<std::path::PathBuf> {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in url.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let key = format!("{:016x}", hash);
    let dir = cache_root?.join("lua-http");
    Some(dir.join(format!("{}.txt", key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job() -> (mpsc::Sender<String>, LuaJob) {
        let (tx, rx) = mpsc::channel::<String>();
        let job = LuaJob {
            rx,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        (tx, job)
    }

    #[test]
    fn parse_request_spec_reads_method_headers_form() {
        let lua = mlua::Lua::new();
        let opts: mlua::Table = lua
            .load(
                r#"return {
                    method  = "POST",
                    headers = { Authorization = "Bearer tok" },
                    form    = { grant_type = "client_credentials" },
                }"#,
            )
            .eval()
            .expect("eval opts");
        let spec = parse_request_spec(Some(opts)).expect("parse");
        assert_eq!(spec.method, "POST");
        assert_eq!(
            spec.headers,
            vec![("Authorization".to_string(), "Bearer tok".to_string())]
        );
        assert_eq!(
            spec.form,
            vec![("grant_type".to_string(), "client_credentials".to_string())]
        );
    }

    #[test]
    fn parse_request_spec_none_is_plain_get() {
        let spec = parse_request_spec(None).expect("parse");
        assert_eq!(spec.method, "");
        assert!(spec.headers.is_empty());
        assert!(spec.form.is_empty());
    }

    #[test]
    fn parse_request_spec_rejects_unknown_method() {
        let lua = mlua::Lua::new();
        let opts: mlua::Table = lua
            .load(r#"return { method = "DELETE" }"#)
            .eval()
            .expect("eval opts");
        assert!(parse_request_spec(Some(opts)).is_err());
    }

    #[test]
    fn job_try_take_returns_nil_before_send() {
        // Build a job by hand (skip the HTTP path) so we can
        // assert try_take's non-blocking behaviour.
        let (tx, job) = make_job();
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

    #[test]
    fn cancel_makes_subsequent_try_take_return_nil_even_with_buffered_body() {
        // Race scenario: worker already enqueued a body before the
        // caller cancelled. `cancel()` must still look like a clean
        // disposal — the buffered body becomes unreachable from
        // `try_take`, regardless of timing.
        let (tx, job) = make_job();
        let lua = mlua::Lua::new();
        let ud = lua.create_userdata(job).expect("create_userdata");

        // Worker sends, then caller cancels — a tight race
        // collapsed onto a single thread for determinism.
        tx.send("late".to_string()).unwrap();
        lua.load("select(1, ...):cancel()")
            .call::<()>(ud.clone())
            .expect("cancel");

        let result: Option<String> = lua
            .load("return select(1, ...):try_take()")
            .call(ud.clone())
            .expect("call");
        assert!(
            result.is_none(),
            "cancelled job must report no body even if buffered"
        );

        // Calling cancel twice is harmless.
        lua.load("select(1, ...):cancel()")
            .call::<()>(ud)
            .expect("second cancel");
    }
}
