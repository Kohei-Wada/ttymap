//! Shared HTTP client.
//!
//! All outbound HTTP in the crate goes through [`HttpClient`] so the
//! user-agent, timeout, and error-logging shape stay uniform. Each
//! domain (nominatim, wiki, tile, …) constructs its own instance with
//! a `tag` used as the log prefix, then calls `get_json` / `get_bytes`.

pub mod rate_limit;
pub mod url;

use std::fmt;
use std::thread;
use std::time::Duration;

use log::debug;
use serde::de::DeserializeOwned;

/// Cap for `Retry-After`-driven retries. We honour the upstream's
/// requested wait once; if the *retry* also returns 429/503 we
/// surface the error rather than loop indefinitely. One honour is
/// enough to absorb the common "you broke a token-bucket window"
/// case without giving a sustained-overload upstream a way to
/// pin our worker thread.
const MAX_RETRY_AFTER_HONOURS: u32 = 1;

/// Fallback sleep when `Retry-After` is absent / unparseable. Long
/// enough that we don't immediately retry on the same window, short
/// enough that the user isn't waiting on something that's never
/// coming back.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);

/// Hard ceiling on `Retry-After` waits. An adversarial server
/// asking us to sleep for an hour would otherwise pin a worker
/// thread; we cap at a value the user might plausibly tolerate
/// before they give up on the fetch.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

const USER_AGENT: &str = concat!(
    "ttymap/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Kohei-Wada/ttymap)"
);

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Classified failure from an HTTP fetch. Callers match on the variant
/// to decide retry / negative-cache / surface-to-UI policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// reqwest IO-level: timeout, DNS, connection refused, mid-body disconnect.
    /// Usually transient — worth retrying with backoff.
    Network(String),
    /// Non-2xx HTTP status. May be permanent (404) or transient (503).
    Http(u16),
    /// Deserialisation failure (JSON / UTF-8). Usually indicates an
    /// upstream schema change — retry will not help.
    Parse(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "network: {msg}"),
            Self::Http(code) => write!(f, "http {code}"),
            Self::Parse(msg) => write!(f, "parse: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::blocking::Client,
    tag: &'static str,
}

impl HttpClient {
    pub fn new(tag: &'static str) -> Result<Self, crate::EngineError> {
        Self::with_timeout(tag, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(tag: &'static str, timeout: Duration) -> Result<Self, crate::EngineError> {
        let inner = builder()
            .timeout(timeout)
            .build()
            .map_err(crate::EngineError::HttpInit)?;
        Ok(Self { inner, tag })
    }

    /// GET + deserialize as JSON. Low-level `debug!` is emitted per
    /// failure so offline debugging has URL + tag context; callers
    /// get a classified [`FetchError`] for policy decisions.
    ///
    /// Rate-limited per third-party-API ToS — see
    /// [`rate_limit`] for the host registry. 429 / 503 responses
    /// are honoured once via the `Retry-After` header; further
    /// rate-limit responses surface as [`FetchError::Http`].
    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, FetchError> {
        let response = self.send_with_retry(url)?;
        response.json().map_err(|e| {
            debug!("{}: {}: parse error: {}", self.tag, url, e);
            FetchError::Parse(e.to_string())
        })
    }

    /// GET + raw bytes. Body-streaming failures are reported as
    /// [`FetchError::Network`] (mid-stream disconnects are networky).
    /// Same rate-limiting / retry policy as [`Self::get_json`].
    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let response = self.send_with_retry(url)?;
        response.bytes().map(|b| b.to_vec()).map_err(|e| {
            debug!("{}: {}: body error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })
    }

    /// Single GET attempt, post-rate-limit. Returns the success
    /// response or maps the error into [`FetchError`] with the same
    /// debug-log shape as before.
    fn send_once(&self, url: &str) -> Result<reqwest::blocking::Response, FetchError> {
        rate_limit::limiter().acquire_for(url);
        let response = self.inner.get(url).send().map_err(|e| {
            debug!("{}: {}: request error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })?;
        if !response.status().is_success() {
            let status = response.status();
            debug!("{}: {}: status {}", self.tag, url, status);
            return Err(FetchError::Http(status.as_u16()));
        }
        Ok(response)
    }

    /// Retry-After-aware wrapper around [`Self::send_once`]. On 429
    /// or 503 we look at the `Retry-After` header (delta-seconds or
    /// HTTP-date), sleep up to [`MAX_RETRY_AFTER`], and retry once.
    /// Any other error short-circuits.
    fn send_with_retry(&self, url: &str) -> Result<reqwest::blocking::Response, FetchError> {
        let mut honours = 0;
        loop {
            match self.send_once(url) {
                Ok(r) => return Ok(r),
                Err(FetchError::Http(status))
                    if (status == 429 || status == 503) && honours < MAX_RETRY_AFTER_HONOURS =>
                {
                    // We can't read `Retry-After` from a `FetchError::Http`
                    // (it discards the response), so re-issue the
                    // request through reqwest directly to peek the
                    // header before sleeping. Slightly wasteful but
                    // keeps `send_once`'s shape simple, and the
                    // re-issue happens only on the rate-limit path.
                    let wait = self
                        .peek_retry_after(url, status)
                        .unwrap_or(DEFAULT_RETRY_AFTER);
                    let wait = wait.min(MAX_RETRY_AFTER);
                    debug!(
                        "{}: {}: {} → honouring Retry-After {:?}",
                        self.tag, url, status, wait
                    );
                    thread::sleep(wait);
                    honours += 1;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Re-issue a HEAD-equivalent fetch only to read `Retry-After`.
    /// Returns `None` if the header is absent or unparseable; the
    /// caller falls back to [`DEFAULT_RETRY_AFTER`]. We do a GET
    /// (not HEAD) because some servers reject HEAD on rate-limited
    /// endpoints — and we already paid the rate-limit cost the
    /// first time.
    fn peek_retry_after(&self, url: &str, expected_status: u16) -> Option<Duration> {
        rate_limit::limiter().acquire_for(url);
        let response = self.inner.get(url).send().ok()?;
        if response.status().as_u16() != expected_status {
            return None;
        }
        let header = response.headers().get(reqwest::header::RETRY_AFTER)?;
        let raw = header.to_str().ok()?;
        // Delta-seconds form (the common case for token-bucket APIs).
        if let Ok(secs) = raw.trim().parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }
        // HTTP-date form. We don't pull a date parser for this; if
        // the server insists on a date format the fallback is fine.
        None
    }
}

fn builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder().user_agent(USER_AGENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_error_display_network() {
        assert_eq!(
            FetchError::Network("timeout".into()).to_string(),
            "network: timeout"
        );
    }

    #[test]
    fn fetch_error_display_http() {
        assert_eq!(FetchError::Http(404).to_string(), "http 404");
    }

    #[test]
    fn fetch_error_display_parse() {
        assert_eq!(
            FetchError::Parse("missing field".into()).to_string(),
            "parse: missing field"
        );
    }
}
