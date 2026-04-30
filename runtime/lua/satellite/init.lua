-- Satellite tracker — single panel showing N satellites at once.
--
-- The sibling `satellite.satellites` factory builds one Component
-- that aggregates every configured sat. In-panel key chars (`i`, `h`
-- …) toggle individual visibility; `Enter` recentres on the first
-- visible one. To track an extra satellite, copy this file to
-- `~/.config/ttymap/lua/satellite.lua` and append its NORAD ID +
-- a free key char.

ttymap.register_plugin(require("satellite.satellites").make({
    -- Manned LEO stations.
    { display = "ISS",      norad_id = 25544, color = "accent_alt", key = "i" },
    { display = "Tiangong", norad_id = 48274, color = "highlight",  key = "T" },

    -- Telescopes.
    { display = "Hubble",   norad_id = 20580, color = "accent",     key = "H" },

    -- Polar / sun-synchronous Earth observation. They sweep the
    -- planet pole-to-pole rather than tracing a single band.
    { display = "Landsat",  norad_id = 49260, color = "link",       key = "L" },
    { display = "Terra",    norad_id = 25994, color = "muted",      key = "R" },
    { display = "Aqua",     norad_id = 27424, color = "link",       key = "A" },
    { display = "NOAA-20",  norad_id = 43013, color = "body",       key = "N" },

    -- Deep elliptical orbits — altitude swings dramatically across
    -- the period; the marker visibly speeds up at perigee and dawdles
    -- at apogee, half a planet's worth of map travel per pass.
    { display = "Chandra",  norad_id = 25867, color = "accent_alt", key = "C" },
    { display = "TESS",     norad_id = 43435, color = "accent",     key = "E" },

    -- MEO (~20,000 km, 12 h period). Slow drift across the map,
    -- a different rhythm to the LEO sats.
    { display = "GPS",      norad_id = 26360, color = "highlight",  key = "G" },
}))
