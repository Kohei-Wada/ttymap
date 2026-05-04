-- Bundled defaults for ttymap. Runs first in the init.lua chain;
-- the user's `~/.config/ttymap/init.lua` runs after this in the
-- same Lua state and can override anything set here (last-wins on
-- the shared `ttymap.opt.*` table).
--
-- Every value below is the Rust-side default. Source of truth:
-- `src/config.rs`. Edit a line to change the shipping default
-- (lands via PR); users override per-leaf in their own init.lua.

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
-- ttymap.opt.disable — opt-out plugin list. Stems are file
-- names under any `<runtime>/plugin/` minus `.lua`. Users
-- should `table.insert(ttymap.opt.disable, "X")` rather than
-- reassign, otherwise bundled entries get clobbered.
------------------------------------------------------------
ttymap.opt.disable = {}

------------------------------------------------------------
-- ttymap.opt.runtime — event-loop / overlay redraw rates.
------------------------------------------------------------
ttymap.opt.runtime.poll_timeout_ms   = 50   -- Main loop wake interval (20 Hz).
ttymap.opt.runtime.overlay_redraw_ms = 100  -- Min interval between overlay-driven redraws (10 Hz).
