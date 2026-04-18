//! Shared HTTP client.
//!
//! All outbound HTTP in the crate goes through [`HttpClient`] so the
//! user-agent, timeout, and error-logging shape stay uniform. Each
//! domain (nominatim, wiki, tile, …) constructs its own instance with
//! a `tag` used as the log prefix, then calls `get_json` / `get_bytes`.

pub mod url;

use std::time::Duration;

use log::debug;
use serde::de::DeserializeOwned;

const USER_AGENT: &str = concat!(
    "ttymap/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Kohei-Wada/ttymap)"
);

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::blocking::Client,
    tag: &'static str,
}

impl HttpClient {
    pub fn new(tag: &'static str) -> Self {
        Self::with_timeout(tag, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(tag: &'static str, timeout: Duration) -> Self {
        let inner = builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client build");
        Self { inner, tag }
    }

    /// GET + deserialize as JSON. Returns `None` and logs on any failure.
    pub fn get_json<T: DeserializeOwned>(&self, url: &str) -> Option<T> {
        let response = match self.inner.get(url).send() {
            Ok(r) => r,
            Err(e) => {
                debug!("{}: {}: request error: {}", self.tag, url, e);
                return None;
            }
        };
        if !response.status().is_success() {
            debug!("{}: {}: status {}", self.tag, url, response.status());
            return None;
        }
        match response.json() {
            Ok(j) => Some(j),
            Err(e) => {
                debug!("{}: {}: parse error: {}", self.tag, url, e);
                None
            }
        }
    }

    /// GET + raw bytes. Returns `None` and logs on any failure.
    pub fn get_bytes(&self, url: &str) -> Option<Vec<u8>> {
        let response = match self.inner.get(url).send() {
            Ok(r) => r,
            Err(e) => {
                debug!("{}: {}: request error: {}", self.tag, url, e);
                return None;
            }
        };
        if !response.status().is_success() {
            debug!("{}: {}: status {}", self.tag, url, response.status());
            return None;
        }
        match response.bytes() {
            Ok(b) => Some(b.to_vec()),
            Err(e) => {
                debug!("{}: {}: body error: {}", self.tag, url, e);
                None
            }
        }
    }
}

fn builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder().user_agent(USER_AGENT)
}
