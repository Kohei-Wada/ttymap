//! Shared HTTP client builder.
//!
//! All outbound HTTP goes through [`client_builder`] so user-agent and
//! version string stay consistent. Callers can override individual
//! settings (e.g. `.timeout(...)`) before `.build()`.

use std::time::Duration;

const USER_AGENT: &str = concat!(
    "ttymap/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/Kohei-Wada/ttymap)"
);

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Returns a blocking `reqwest` client builder pre-configured with the
/// crate's user-agent and a default timeout. Call `.build()` to get a
/// client, or override fields first.
pub fn client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(DEFAULT_TIMEOUT)
}
