//! Process-wide HTTP rate limiter.
//!
//! ttymap fans out HTTP fetches from many places (engine tile
//! pipeline, every Lua plugin via `ttymap.http:fetch`, geoip
//! resolver, …). Each clones an [`HttpClient`] but they all need to
//! respect a *global* per-host cap — Nominatim's policy is "1
//! request / second across all of your code", not "1 / second per
//! consumer". So the bucket map lives in a process-wide singleton,
//! not per `HttpClient` instance.
//!
//! Default registry covers the two upstreams ttymap actively
//! disagrees with by ToS:
//!
//! - `nominatim.openstreetmap.org` — **1 rps** (OSM Foundation
//!   policy, see <https://operations.osmfoundation.org/policies/nominatim/>).
//! - `*.wikipedia.org` / `*.wikimedia.org` — **5 rps** as a self-
//!   imposed safety margin; Wikipedia has no hard QPS but bursts
//!   over ~200/s from one IP risk a temporary block.
//!
//! Hosts that don't match any registered suffix pass through with no
//! rate limit. mapscii.me tile fetches are intentionally not in the
//! registry — the upstream has no published cap and the worker pool
//! already bounds in-flight count.
//!
//! A user pointing at a private Nominatim (e.g. via the search
//! plugin's Lua-side `require("ttymap.search").endpoint = "..."`)
//! automatically falls through the registry and runs unthrottled —
//! that's the right default for self-hosted infrastructure.
//!
//! Implementation is a token bucket per host: `acquire_for` blocks
//! until a token is available, then consumes one. Refill happens
//! lazily on each `acquire` call; no background thread.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// One token bucket. `acquire` blocks (with `thread::sleep`) until
/// `tokens >= 1`, then decrements. Lazy refill keeps the bucket
/// thread-light — no timer, no background work.
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        if dt > 0.0 {
            self.tokens = (self.tokens + dt * self.refill_per_sec).min(self.capacity);
            self.last_refill = now;
        }
    }

    /// How long until the bucket has at least one token, given the
    /// current tokens count and refill rate. Returns zero when a
    /// token is available now.
    fn time_until_token(&self) -> Duration {
        if self.tokens >= 1.0 {
            return Duration::ZERO;
        }
        let needed = 1.0 - self.tokens;
        let secs = needed / self.refill_per_sec;
        Duration::from_secs_f64(secs)
    }

    fn consume(&mut self) {
        self.tokens -= 1.0;
    }
}

/// Per-host registry. `Mutex<HashMap<...>>` rather than a lock-free
/// map because the hot path is "one bucket, many hits" — contention
/// is on the bucket, not the map.
pub struct RateLimiter {
    by_suffix: Mutex<HashMap<&'static str, Mutex<TokenBucket>>>,
}

impl RateLimiter {
    fn empty() -> Self {
        Self {
            by_suffix: Mutex::new(HashMap::new()),
        }
    }

    fn with_defaults() -> Self {
        let r = Self::empty();
        r.register("nominatim.openstreetmap.org", 1.0, 1.0);
        // 5 rps for Wikipedia / Wikimedia — capacity 5 lets a burst
        // through for the wiki plugin's geosearch + extracts pair
        // (two hits per refresh) without per-call queueing.
        r.register("wikipedia.org", 5.0, 5.0);
        r.register("wikimedia.org", 5.0, 5.0);
        r
    }

    /// Register (or replace) a token bucket for a host suffix. The
    /// suffix matches with a leading-dot or full-equality check, so
    /// `"wikipedia.org"` covers `en.wikipedia.org`, `ja.wikipedia.org`,
    /// and the bare host.
    pub fn register(&self, suffix: &'static str, capacity: f64, refill_per_sec: f64) {
        let mut map = self.by_suffix.lock().expect("rate-limiter map poisoned");
        map.insert(
            suffix,
            Mutex::new(TokenBucket::new(capacity, refill_per_sec)),
        );
    }

    /// Block until a token is available for the host of `url`. If no
    /// suffix matches, returns immediately (no rate limit).
    pub fn acquire_for(&self, url: &str) {
        let host = match host_from_url(url) {
            Some(h) => h,
            None => return,
        };
        let map = self.by_suffix.lock().expect("rate-limiter map poisoned");
        let suffix = map.keys().copied().find(|s| host_matches(host, s));
        let suffix = match suffix {
            Some(s) => s,
            None => return,
        };
        // Drop the outer map lock before sleeping on the inner
        // bucket — otherwise an N-rps bucket on host A would stall
        // unrelated host B's lookup. The two-phase lock is the
        // whole reason the inner bucket has its own Mutex.
        //
        // We can't hold a reference into `map` past the drop, so
        // copy the suffix key out and re-look up the bucket on a
        // fresh borrow each spin. `register()` is rare and only
        // adds entries, so the ref stays valid; the cost of the
        // re-lookup is one HashMap probe per ~tens of milliseconds
        // of sleep, dwarfed by the sleep itself.
        drop(map);
        loop {
            let map = self.by_suffix.lock().expect("rate-limiter map poisoned");
            let bucket_mutex = match map.get(suffix) {
                Some(b) => b,
                None => return, // entry was somehow removed — give up the limit
            };
            let mut bucket = bucket_mutex.lock().expect("rate-limiter bucket poisoned");
            bucket.refill();
            if bucket.tokens >= 1.0 {
                bucket.consume();
                return;
            }
            let wait = bucket.time_until_token();
            // Drop both locks before sleeping so other hosts can
            // make progress.
            drop(bucket);
            drop(map);
            thread::sleep(wait);
        }
    }
}

/// Extract the host segment of a URL. Lightweight parser — we don't
/// pull `url::Url` for one field. Returns `None` for inputs that
/// don't look like an absolute URL (relative paths, malformed input);
/// callers treat that as "no rate limit applies" because there's no
/// host to bucket on.
fn host_from_url(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://")?.1;
    let host_with_path = after_scheme
        .split_once('/')
        .map_or(after_scheme, |(h, _)| h);
    // Strip optional userinfo (user:pass@host) and port (host:port).
    let host_with_port = host_with_path
        .rsplit_once('@')
        .map_or(host_with_path, |(_, h)| h);
    let host = host_with_port
        .split_once(':')
        .map_or(host_with_port, |(h, _)| h);
    if host.is_empty() { None } else { Some(host) }
}

/// `host_matches("en.wikipedia.org", "wikipedia.org")` → true;
/// `host_matches("evilwikipedia.org", "wikipedia.org")` → false.
/// The dot guard prevents the substring trick where a malicious host
/// suffix-matches a legitimate one without sharing the actual domain
/// boundary.
fn host_matches(host: &str, suffix: &str) -> bool {
    if host == suffix {
        return true;
    }
    if host.len() <= suffix.len() {
        return false;
    }
    let split = host.len() - suffix.len();
    host.as_bytes().get(split - 1) == Some(&b'.') && &host[split..] == suffix
}

/// Process-wide singleton. Initialised lazily on first `acquire_for`.
static RATE_LIMITER: OnceLock<RateLimiter> = OnceLock::new();

pub fn limiter() -> &'static RateLimiter {
    RATE_LIMITER.get_or_init(RateLimiter::with_defaults)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn host_from_url_extracts_host() {
        assert_eq!(
            host_from_url("https://example.com/path"),
            Some("example.com")
        );
        assert_eq!(
            host_from_url("http://en.wikipedia.org/wiki/X"),
            Some("en.wikipedia.org")
        );
        assert_eq!(host_from_url("https://host:8080/"), Some("host"));
        assert_eq!(
            host_from_url("https://user:pw@host.example/"),
            Some("host.example")
        );
        assert_eq!(
            host_from_url("https://only-host-no-path"),
            Some("only-host-no-path")
        );
        assert_eq!(host_from_url("not-a-url"), None);
        assert_eq!(host_from_url("///path-only"), None);
    }

    #[test]
    fn host_matches_requires_dot_boundary() {
        assert!(host_matches("wikipedia.org", "wikipedia.org"));
        assert!(host_matches("en.wikipedia.org", "wikipedia.org"));
        assert!(host_matches("a.b.wikipedia.org", "wikipedia.org"));
        // The dot-guard rejects this one — substring match without
        // a real domain boundary.
        assert!(!host_matches("evilwikipedia.org", "wikipedia.org"));
        assert!(!host_matches("wikipedia.org.evil.com", "wikipedia.org"));
        assert!(!host_matches("notthesame.org", "wikipedia.org"));
        assert!(!host_matches("short", "wikipedia.org"));
    }

    #[test]
    fn token_bucket_acquire_returns_immediately_when_token_available() {
        let r = RateLimiter::empty();
        r.register("example.com", 5.0, 1.0); // 5 burst, 1 rps refill
        let start = Instant::now();
        r.acquire_for("https://example.com/x");
        r.acquire_for("https://example.com/x");
        r.acquire_for("https://example.com/x");
        // Three immediate acquisitions out of capacity 5 should be
        // effectively instant — tolerate up to 50 ms of scheduler noise.
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "elapsed = {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn token_bucket_acquire_blocks_when_empty() {
        // capacity 1, refill 5 rps → 200 ms between tokens. After
        // consuming the initial token, the next acquire must wait
        // ~200 ms.
        let r = RateLimiter::empty();
        r.register("example.com", 1.0, 5.0);
        r.acquire_for("https://example.com/x"); // consume initial token
        let start = Instant::now();
        r.acquire_for("https://example.com/x");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(150),
            "expected ~200 ms wait, got {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(400),
            "wait was longer than expected: {:?}",
            elapsed
        );
    }

    #[test]
    fn unregistered_host_passes_through() {
        let r = RateLimiter::empty();
        r.register("example.com", 1.0, 0.001); // very slow refill, but unrelated
        let start = Instant::now();
        for _ in 0..50 {
            r.acquire_for("https://other-host.invalid/x");
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "elapsed = {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn suffix_matches_subdomains() {
        // Wikipedia rule should cover en.wikipedia.org without
        // a separate registration.
        let r = RateLimiter::empty();
        r.register("wikipedia.org", 1.0, 5.0);
        r.acquire_for("https://en.wikipedia.org/x"); // consume
        let start = Instant::now();
        r.acquire_for("https://ja.wikipedia.org/x");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(150),
            "subdomain should share bucket; elapsed = {:?}",
            elapsed
        );
    }
}
