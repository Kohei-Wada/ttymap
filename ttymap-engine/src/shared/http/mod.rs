//! Shared HTTP client.
//!
//! All outbound HTTP in the crate goes through [`HttpClient`] so the
//! user-agent, timeout, and error-logging shape stay uniform. Each
//! domain (nominatim, wiki, tile, …) constructs its own instance with
//! a `tag` used as the log prefix, then calls `get_json` / `get_bytes`.

pub mod url;

use std::fmt;
use std::time::Duration;

use log::debug;
use serde::de::DeserializeOwned;

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
    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, FetchError> {
        let response = self.inner.get(url).send().map_err(|e| {
            debug!("{}: {}: request error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })?;
        if !response.status().is_success() {
            let status = response.status();
            debug!("{}: {}: status {}", self.tag, url, status);
            return Err(FetchError::Http(status.as_u16()));
        }
        response.json().map_err(|e| {
            debug!("{}: {}: parse error: {}", self.tag, url, e);
            FetchError::Parse(e.to_string())
        })
    }

    /// GET + raw bytes. Body-streaming failures are reported as
    /// [`FetchError::Network`] (mid-stream disconnects are networky).
    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let response = self.inner.get(url).send().map_err(|e| {
            debug!("{}: {}: request error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })?;
        if !response.status().is_success() {
            let status = response.status();
            debug!("{}: {}: status {}", self.tag, url, status);
            return Err(FetchError::Http(status.as_u16()));
        }
        response.bytes().map(|b| b.to_vec()).map_err(|e| {
            debug!("{}: {}: body error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })
    }

    /// General request → raw bytes. `method` is `"POST"` for a POST
    /// (any other value is a GET); `headers` are extra request headers
    /// (e.g. `Authorization`); a non-empty `form` is sent as an
    /// `application/x-www-form-urlencoded` body. Backs the Lua
    /// `ttymap.http:fetch(url, opts)` surface so plugins can do auth'd /
    /// token-exchange requests (OAuth2 client-credentials, …) without
    /// any endpoint-specific Rust.
    pub fn request_bytes(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        form: &[(String, String)],
    ) -> Result<Vec<u8>, FetchError> {
        let mut req = if method.eq_ignore_ascii_case("POST") {
            self.inner.post(url)
        } else {
            self.inner.get(url)
        };
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if !form.is_empty() {
            req = req.form(form);
        }
        let response = req.send().map_err(|e| {
            debug!("{}: {}: request error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })?;
        if !response.status().is_success() {
            let status = response.status();
            debug!("{}: {}: status {}", self.tag, url, status);
            return Err(FetchError::Http(status.as_u16()));
        }
        response.bytes().map(|b| b.to_vec()).map_err(|e| {
            debug!("{}: {}: body error: {}", self.tag, url, e);
            FetchError::Network(e.to_string())
        })
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
