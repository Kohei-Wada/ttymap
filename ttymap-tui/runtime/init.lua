-- Bundled defaults for ttymap. The order is the standard layered
-- one (system → bundled → user):
--
--   1. seed `ttymap.opt.*` defaults (bundled — shipping values)
--   2. wire bundled libs (`ttymap.notify` etc.) via `setup()`
--   3. activate the bundled plugin set via `require`
--   4. pull in the user's `~/.config/ttymap/init.lua` LAST, so
--      user mutations / handle :remove() / user `require`s win
--
-- The user-config loader is a Lua lib (`ttymap.user_config`); Rust
-- never names the user-config path. To replace the bundled set
-- entirely, point `$TTYMAP_RUNTIME` at your own runtime layer with
-- its own `init.lua`.
--
-- Source of truth for the option defaults below: `src/config.rs`.
-- Editing a line here changes the shipping default (via PR); users
-- override per-leaf in their own init.lua.

------------------------------------------------------------
-- 0. Install the plugin-aware `package.searchers` entry. The
-- host (Rust) does not know about the `<layer>/plugin/...`
-- convention; `ttymap.plugin_searcher` (a Lua lib) owns that
-- resolution, using only the `ttymap.runtime_path` primitive
-- Rust exposes — the chunk runs as a plain `load(source)()`,
-- with `register_*` calls inside pushing directly into the
-- live host registry.
------------------------------------------------------------
require("ttymap.plugin_searcher").install()

------------------------------------------------------------
-- 1. ttymap.opt.map — initial viewport + zoom envelope.
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
-- 2. Bundled libs — infrastructure consumed by every plugin's
-- `ttymap.notify(msg)` calls. A lib (not plugin) so users can
-- pass `setup({ ttl_s = …, ring_cap = …, max_text_width = … })`
-- to tweak the renderer; skipping the call disables it entirely.
------------------------------------------------------------
require("ttymap.notify").setup()

------------------------------------------------------------
-- 3. Bundled plugins — chrome first, then everything else
-- (alphabetical). Adjust per file to taste.
------------------------------------------------------------
require "info"
require "scalebar"
require "attribution"
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

------------------------------------------------------------
-- 4. User init.lua — runs LAST so the user wins:
--   * override any `ttymap.opt.*` set above
--   * `ttymap.keymap.set/del`
--   * `require` user plugins (their registrations stack on top of
--     bundled; the registry scans in registration order, so for a
--     keybind conflict the user must `:remove()` the bundled handle
--     first — bundled plugins don't expose their handles by default,
--     so this is mostly a "use a different keybind" situation)
--   * use the handle returned by your own `register_palette_command`
--     / `register_keybind` / `on_event` and call `:remove()` on it
--     to drop a registration later
-- Missing / broken user file = logged-and-skipped, the host keeps
-- booting.
------------------------------------------------------------
require("ttymap.user_config").load()
