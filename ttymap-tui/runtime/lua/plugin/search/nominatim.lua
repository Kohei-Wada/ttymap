-- search.nominatim — Nominatim forward-geocoding REST client.
--
-- Endpoint is read from `ttymap.search.endpoint` (default points at
-- the public OSM Nominatim instance, rate-limited to 1 rps by the
-- Rust HTTP layer per OSMF policy). Users with a private Nominatim
-- override via `require("ttymap.search").endpoint = "..."` in
-- init.lua; the change takes effect on the next query — no restart.
--
-- Debounce in the search palette keeps the same prefix from being
-- re-fetched per keystroke; the rate-limiter is the global cap.

local config = require "ttymap.search"

local M = {}

local LIMIT = 5

function M.url(query)
    return string.format("%s?q=%s&format=json&limit=%d",
        config.endpoint, ttymap.http:url_encode(query), LIMIT)
end

function M.parse(payload)
    local out = {}
    if type(payload) ~= "table" then return out end
    for _, item in ipairs(payload) do
        if type(item) == "table"
            and type(item.display_name) == "string"
            and type(item.lat) == "string"
            and type(item.lon) == "string" then
            local lat = tonumber(item.lat)
            local lon = tonumber(item.lon)
            if lat and lon then
                table.insert(out, {
                    name = item.display_name,
                    lon = lon,
                    lat = lat,
                })
            end
        end
    end
    return out
end

return M
