//! [`Event`] enum + helper types.
//!
//! Each variant is the typed payload for one observable thing the
//! app does. Variants are `Send` so cross-thread producers can wrap
//! them in [`crate::app::AppEvent::Bus`] and push onto the App mpsc.
//!
//! The string returned by [`Event::name`] is the key Lua plugins use
//! in `ttymap.on_event(name, fn)` — keep it stable across releases.

/// User-visible severity for [`Event::Notify`]. Renderers map this
/// to colour / emphasis; the bus carries no display policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    /// Stable string form crossing the Lua boundary
    /// (`{ level = "info" | "warn" | "error" }`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Permissive parse — anything we don't recognise becomes
    /// [`Level::Info`] so a typo still surfaces.
    pub fn parse(s: &str) -> Self {
        match s {
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// Observational events broadcast by the dispatcher (and other
/// producers). Subscribers — Rust closures or Lua plugin functions —
/// react after the state mutation already happened.
#[derive(Clone, Debug)]
pub enum Event {
    /// Transient status message for the user. Producers on either
    /// side of the Lua boundary fire this; the bundled `notify.lua`
    /// renderer subscribes and paints recent ones top-left for ~3s.
    Notify { message: String, level: Level },
}

impl Event {
    /// Stable Lua-facing name. Same key plugins pass to
    /// `ttymap.on_event(name, fn)`. Bake the spelling here so every
    /// emit site agrees — Lua scripts use bare strings out of
    /// necessity, but everything inside Rust goes through this.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Notify { .. } => "notify",
        }
    }
}
