//! [`Event`] enum + helper types.
//!
//! Each variant is the typed payload for one observable thing the
//! app does. Variants are `Send` so cross-thread producers can wrap
//! them in [`crate::app::AppEvent::Bus`] and push onto the App mpsc.
//!
//! The string returned by [`Event::name`] is the key Lua plugins use
//! in `ttymap.on_event(name, fn)` — keep it stable across releases.

use ttymap_engine::geo::LonLat;

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
    /// A freshly rendered [`MapFrame`](ttymap_engine::map::render::frame::MapFrame)
    /// arrived from the render thread. No payload — the live frame
    /// is reachable through `ttymap.map` accessors.
    FrameReady,
    /// Map recentred via [`MapAction::Jump`](ttymap_engine::map::MapAction::Jump).
    /// Payload: new centre.
    MapJumped(LonLat),
    /// Direct zoom set via [`MapAction::SetZoom`](ttymap_engine::map::MapAction::SetZoom).
    /// Payload: new zoom level.
    MapZoomSet(f64),
    /// Composite recentre+zoom via [`MapAction::FlyTo`](ttymap_engine::map::MapAction::FlyTo).
    /// Payload: new centre, new zoom.
    MapFlewTo(LonLat, f64),
    /// Active theme switched. Payload: theme name (`"dark"` / `"bright"`).
    ThemeChanged(String),
    /// Terminal resized. Payload: `(cols, rows)`.
    Resized(u16, u16),
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
            Self::FrameReady => "frame_ready",
            Self::MapJumped(_) => "map_jumped",
            Self::MapZoomSet(_) => "map_zoom_set",
            Self::MapFlewTo(_, _) => "map_flew_to",
            Self::ThemeChanged(_) => "theme_changed",
            Self::Resized(_, _) => "resized",
            Self::Notify { .. } => "notify",
        }
    }
}
