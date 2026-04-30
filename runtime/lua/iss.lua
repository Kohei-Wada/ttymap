-- ISS — International Space Station tracker.
--
-- TLE-driven via `ttymap.sgp4`; the previous Open Notify HTTP poll
-- is gone. See `runtime/lua/ttymap/satellites.lua` for the shared
-- factory; this file is a one-line spec.

return require("ttymap.satellites").make({
    display = "ISS",
    norad_id = 25544,
    color = "accent_alt",
})
