-- aircraft.opensky — OpenSky Network REST API client.
--
-- Source: https://opensky-network.org/api/states/all
-- Anonymous credit cost: 1 per call when a bbox is supplied (this
-- module always supplies one — the user's view ± `BBOX_HALF_DEG`),
-- 4 without. Refresh cadence (`INTERVAL_SEC`) matches the original
-- Rust plugin so OpenSky's per-IP daily budget lasts.
--
-- State-vector indices follow the OpenSky doc:
--   1 = icao24, 2 = callsign, 6 = lon, 7 = lat,
--   8 = baro_altitude, 9 = on_ground, 10 = velocity, 11 = true_track
-- (Lua arrays are 1-indexed; OpenSky's docs are 0-indexed.)

local M = {}

local BASE_URL       = "https://opensky-network.org/api/states/all"
local BBOX_HALF_DEG  = 5.0   -- half-side of the bbox sent per fetch

M.INTERVAL_SEC = 12

local function trim(s)
    return (s or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function clamp(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

-- bbox URL around (lon, lat). Latitude clamps to [-90, 90],
-- longitude to [-180, 180]; OpenSky doesn't accept antimeridian-
-- wrapping bboxes so this just stops at the edge.
function M.url(lon, lat)
    local lamin = clamp(lat - BBOX_HALF_DEG, -90.0, 90.0)
    local lamax = clamp(lat + BBOX_HALF_DEG, -90.0, 90.0)
    local lomin = clamp(lon - BBOX_HALF_DEG, -180.0, 180.0)
    local lomax = clamp(lon + BBOX_HALF_DEG, -180.0, 180.0)
    return string.format(
        "%s?lamin=%f&lomin=%f&lamax=%f&lomax=%f",
        BASE_URL, lamin, lomin, lamax, lomax
    )
end

function M.parse(payload)
    local out = {}
    if not payload or not payload.states then return out end
    for _, s in ipairs(payload.states) do
        -- Skip rows missing lon/lat (fresh tracks before first
        -- position lock).
        local lon, lat = s[6], s[7]
        if type(lon) == "number" and type(lat) == "number" then
            table.insert(out, {
                callsign  = trim(s[2]),
                lon       = lon,
                lat       = lat,
                on_ground = s[9] == true,
                alt       = s[8],
                heading   = s[11],
            })
        end
    end
    return out
end

return M
