-- Satellite tracker — single panel showing N satellites at once.
--
-- The shared `ttymap.satellites` factory builds one Component that
-- aggregates every configured sat. In-panel key chars (`i`, `h` …)
-- toggle individual visibility; `Enter` recentres on the first
-- visible one. To track an extra satellite, copy this file to
-- `~/.config/ttymap/lua/satellite.lua` and append its NORAD ID +
-- a free key char.

return require("ttymap.satellites").make({
    { display = "ISS",    norad_id = 25544, color = "accent_alt", key = "i" },
    { display = "Hubble", norad_id = 20580, color = "accent",     key = "h" },
})
