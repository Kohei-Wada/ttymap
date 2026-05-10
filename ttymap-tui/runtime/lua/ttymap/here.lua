-- ttymap.here — config holder for the bundled `here` plugin.
--
-- nvim-style Lua-side seam: this module returns one cached table;
-- the plugin requires the same table on activation, so init.lua
-- pre-pass mutations are visible:
--
--     -- ~/.config/ttymap/init.lua
--     require("ttymap.here").endpoint = "https://my-geoip.example.com/json"
--
-- The plugin reads `endpoint` on every invocation, so the override
-- applies on the next "Jump to here" without restart.
--
-- Default endpoint is `https://ipapi.co/json/`, which returns
-- `{"latitude": <f64>, "longitude": <f64>, ...}`. Any service that
-- follows the same key shape works as a drop-in replacement.

return {
    endpoint = "https://ipapi.co/json/",
}
