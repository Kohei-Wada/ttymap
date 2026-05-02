-- search.nominatim — Nominatim forward-geocoding REST client.
--
-- Source: https://nominatim.openstreetmap.org/search
-- Free public endpoint; debounce keeps it from being hammered while
-- the user types.

local M = {}

local SEARCH_URL = "https://nominatim.openstreetmap.org/search"
local LIMIT = 5

function M.url(query)
    return string.format("%s?q=%s&format=json&limit=%d",
        SEARCH_URL, ttymap.http:url_encode(query), LIMIT)
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
