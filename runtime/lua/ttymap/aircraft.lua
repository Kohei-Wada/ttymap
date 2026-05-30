-- ttymap.aircraft — config holder for the bundled `aircraft` plugin.
--
-- OpenSky moved to OAuth2 client-credentials in 2025; anonymous access
-- is capped at 400 credits/day (a ~10°×10° /states/all call costs ~2),
-- so frequent use gets throttled fast. Authenticate to lift the cap to
-- 4000/day (8000 if you feed the network).
--
-- Set your credentials in `~/.config/ttymap/init.lua`:
--
--   local aircraft = require("ttymap.aircraft")
--   aircraft.client_id     = "your-client-id"
--   aircraft.client_secret = "your-client-secret"
--
-- Create an API client at https://opensky-network.org → Account → API
-- clients. Leave both unset for anonymous access (the default).
--
-- nvim-style: the plugin reads these fields lazily at fetch time, so
-- setting them in init.lua (which runs after the plugin is required)
-- takes effect on the next refresh.
--
-- `max_count` caps how many aircraft are shown — dense airspace can
-- return hundreds. When set, only the nearest `max_count` to the map
-- centre are kept (markers + sidebar). `nil` = no cap (show all).
--
--   require("ttymap.aircraft").max_count = 50
--
-- `interval_sec` overrides the refresh cadence. Default auto-picks by
-- auth state — 5 s authenticated (OpenSky's 5 s resolution), 12 s
-- anonymous. Lower burns credits faster; faster than the resolution
-- just re-fetches identical data.
local M = {
    client_id = nil,
    client_secret = nil,
    max_count = nil,
    interval_sec = nil,
}

return M
