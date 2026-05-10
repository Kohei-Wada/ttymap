-- Bundled defaults for ttymap. Runs first; calls into the user's
-- `~/.config/ttymap/init.lua` partway through (via the Rust-provided
-- `ttymap.load_user_config()`) so user mutations land BEFORE bundled
-- plugins are required. Both files share one Lua VM.
--
-- Every option value is the Rust-side default. Source of truth:
-- `src/config.rs`. Edit a line to change the shipping default
-- (lands via PR); users override per-leaf in their own init.lua.
--
-- Disable a bundled plugin from your user init.lua by pre-marking it
-- as already-loaded — Lua's module cache makes the subsequent
-- `require` a no-op:
--
--     -- ~/.config/ttymap/init.lua
--     package.loaded.quake = true
--     package.loaded.aircraft = true
--
-- Or replace `runtime/init.lua` entirely via `$TTYMAP_RUNTIME` for
-- a fully custom plugin set.

------------------------------------------------------------
-- ttymap.opt.map — initial viewport + zoom envelope.
------------------------------------------------------------
ttymap.opt.map.lat       = 52.51298   -- Berlin
ttymap.opt.map.lon       = 13.42012
ttymap.opt.map.zoom      = nil        -- nil → auto-zoom on startup. Set 0..max_zoom to pin.
ttymap.opt.map.max_zoom  = 18.0       -- Upper bound on user zoom.
ttymap.opt.map.zoom_step = 0.2        -- Per-keypress zoom delta (`+` / `-`).

------------------------------------------------------------
-- ttymap.opt.render — visual theme + label language.
------------------------------------------------------------
ttymap.opt.render.style    = "dark"   -- "dark" | "bright"
ttymap.opt.render.language = "en"     -- MVT label tag suffix, e.g. "en", "ja", "de"

------------------------------------------------------------
-- ttymap.opt.cache — tile cache knobs.
------------------------------------------------------------
ttymap.opt.cache.tiles        = true  -- Persist decoded tiles under ~/.cache/ttymap/
ttymap.opt.cache.memory_tiles = 512   -- Decoded-tile LRU capacity. ~22 tiles per view.

------------------------------------------------------------
-- ttymap.opt.geoip — IP-based location on startup.
------------------------------------------------------------
ttymap.opt.geoip.on_startup = false                       -- Equivalent to passing --here
ttymap.opt.geoip.endpoint   = "https://ipapi.co/json/"    -- Must return ipapi.co-shaped JSON.
ttymap.opt.geoip.timeout_ms = 2000

------------------------------------------------------------
-- ttymap.opt.runtime — event-loop / overlay redraw rates.
------------------------------------------------------------
ttymap.opt.runtime.poll_timeout_ms   = 50   -- Main loop wake interval (20 Hz).
ttymap.opt.runtime.overlay_redraw_ms = 100  -- Min interval between overlay-driven redraws (10 Hz).

------------------------------------------------------------
-- User init.lua — runs HERE so the user can:
--   * override any `ttymap.opt.*` set above
--   * pre-mark `package.loaded.X = true` to skip a bundled plugin
--   * `require` user plugins in any order (before or after bundled)
-- Missing / broken user file = logged-and-skipped, this file
-- continues normally.
------------------------------------------------------------
ttymap.load_user_config()

------------------------------------------------------------
-- Bundled plugins — chrome first, then everything else
-- (alphabetical). Adjust per file to taste.
------------------------------------------------------------
require "info"
require "scalebar"
require "attribution"
require "notify"
require "help"

require "aircraft"
require "center"
require "export"
require "geo_quiz"
require "here"
require "ping_simulation"
require "quake"
require "satellite"
require "search"
require "terminator"
require "travel"
require "wiki"
