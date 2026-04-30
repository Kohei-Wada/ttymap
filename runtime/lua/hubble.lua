-- Hubble Space Telescope tracker.
--
-- Companion to iss.lua; both go through the shared
-- `runtime/lua/ttymap/satellites.lua` factory. Drop a similar one-line
-- file under `~/.config/ttymap/plugins/` with any other NORAD ID to
-- track that satellite (e.g. Tiangong = 48274).

return require("ttymap.satellites").make({
    display = "Hubble",
    norad_id = 20580,
    color = "accent",
})
