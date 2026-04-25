//! Throttled background feed — `Throttle + AsyncJob` rolled together.
//!
//! Every plugin that periodically pulls data from a remote service
//! ends up with the same trio: a [`Throttle`] gating how often a
//! request fires, an [`AsyncJob`] running the request on a fresh
//! thread, and a `try_recv` poll on the main thread to drain the
//! result. [`PolledFeed`] is that trio behind one type so plugins
//! can drop the per-plugin `service.rs` glue.
//!
//! `PolledFeed` deliberately does **not** own the latest data —
//! [`poll`](Self::poll) hands the raw `Option<T>` to the caller so
//! each plugin can choose its own merge strategy (replace
//! wholesale, dedup by id, preserve a selection index, etc.).
//!
//! ## Typical use
//!
//! ```ignore
//! use std::sync::Arc;
//! use std::time::Duration;
//! use crate::shared::polled_feed::PolledFeed;
//!
//! struct State {
//!     client: Arc<MyClient>,
//!     feed:   PolledFeed<Vec<MyItem>>,
//!     items:  Vec<MyItem>,
//! }
//!
//! impl State {
//!     fn refresh(&mut self) {
//!         let client = self.client.clone();
//!         self.feed.refresh(move || client.fetch());
//!     }
//!     fn poll(&mut self) {
//!         if let Some(new) = self.feed.poll() {
//!             self.items = new;
//!         }
//!     }
//! }
//! ```

use std::time::Duration;

use crate::plugin_api::async_job::AsyncJob;
use crate::plugin_api::throttle::Throttle;

pub struct PolledFeed<T: Send + 'static> {
    job: AsyncJob<T>,
    throttle: Throttle,
}

impl<T: Send + 'static> PolledFeed<T> {
    /// Build a feed whose first [`refresh`](Self::refresh) call fires
    /// immediately; subsequent calls only fire once `interval` has
    /// elapsed since the last successful spawn.
    pub fn ready(interval: Duration) -> Self {
        Self {
            job: AsyncJob::new(),
            throttle: Throttle::ready(interval),
        }
    }

    /// Build a feed that starts cooled-down: the first
    /// [`refresh`](Self::refresh) call only fires once `interval` has
    /// elapsed since construction. Useful when state is created at
    /// app start but the first fetch should wait for an explicit
    /// activation gesture (see wiki).
    pub fn with_cooldown(interval: Duration) -> Self {
        Self {
            job: AsyncJob::new(),
            throttle: Throttle::with_cooldown(interval),
        }
    }

    /// Spawn `fetch` on a fresh thread if the throttle allows.
    /// Returns `true` when the spawn fired (caller can use this to log
    /// or surface activity); `false` when the throttle gated it.
    pub fn refresh<F>(&mut self, fetch: F) -> bool
    where
        F: FnOnce() -> T + Send + 'static,
    {
        if self.throttle.check() {
            self.job.spawn(fetch);
            true
        } else {
            false
        }
    }

    /// Drain one completed result, if any. Non-blocking. Caller
    /// stores or merges the value as it sees fit.
    pub fn poll(&mut self) -> Option<T> {
        self.job.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn first_refresh_fires_then_throttles() {
        let mut feed: PolledFeed<u32> = PolledFeed::ready(Duration::from_secs(60));
        assert!(feed.refresh(|| 1), "first refresh should pass");
        assert!(
            !feed.refresh(|| 2),
            "second refresh inside the interval should be gated"
        );
    }

    #[test]
    fn poll_drains_in_order() {
        // Use a tiny interval so two refreshes both fire.
        let mut feed: PolledFeed<u32> = PolledFeed::ready(Duration::from_nanos(1));
        feed.refresh(|| 42);
        // Spin briefly until the spawned thread sends.
        let mut got = None;
        for _ in 0..1000 {
            if let Some(v) = feed.poll() {
                got = Some(v);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(got, Some(42));
        assert!(feed.poll().is_none(), "channel should be drained");
    }
}
