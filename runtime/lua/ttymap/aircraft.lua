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
local M = {
    client_id = nil,
    client_secret = nil,
}

return M
