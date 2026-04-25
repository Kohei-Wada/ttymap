//! Plugin API — opt-in toolbox for plugin authors.
//!
//! Distinct from the **plugin trait** ([`compositor::Component`] and
//! friends) which is the *contract* the framework calls into. The
//! contents here are the *services* a plugin chooses to call out to:
//! cross-cutting helpers that several plugins want but no single
//! subsystem owns.
//!
//! Subsystem-specific plugin surfaces (e.g. [`MapApi`](crate::map::MapApi)
//! for drawing on the map) live with their owning subsystem; this
//! module is for the cross-cutting "stdlib" of plugin authoring.
//!
//! ## Available helpers
//!
//! - [`PolledFeed`] — `Throttle + AsyncJob` rolled together for
//!   live-data plugins (aircraft / ISS / quake / wiki / search all
//!   share the same shape).

pub mod polled_feed;

pub use polled_feed::PolledFeed;
