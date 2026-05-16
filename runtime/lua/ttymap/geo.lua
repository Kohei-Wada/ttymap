-- ttymap.geo — geographic / great-circle math helpers shared by
-- plugins that need to reason about distance, bearing, or the
-- antipode of a point on the WGS-84 sphere.
--
-- All angles are degrees on input and degrees on output; internal
-- conversion to radians stays inside each function. Longitude is
-- antimeridian-aware everywhere — a 179 → -179 step reads as the
-- short way round, not the 40 000 km long way.

local M = {}

M.EARTH_RADIUS_M = 6378137.0   -- WGS-84 equatorial radius

local COMPASS = { "N", "NE", "E", "SE", "S", "SW", "W", "NW" }

local function deg2rad(d)
    return d * math.pi / 180
end

--- Great-circle distance in metres between two (lat, lon) points.
function M.haversine_m(lat1, lon1, lat2, lon2)
    local rlat1 = deg2rad(lat1)
    local rlat2 = deg2rad(lat2)
    local dlat = rlat2 - rlat1
    local dlon = lon2 - lon1
    if dlon > 180 then dlon = dlon - 360
    elseif dlon < -180 then dlon = dlon + 360 end
    dlon = deg2rad(dlon)
    local a = math.sin(dlat / 2) ^ 2
        + math.cos(rlat1) * math.cos(rlat2) * math.sin(dlon / 2) ^ 2
    return 2 * M.EARTH_RADIUS_M * math.asin(math.min(1, math.sqrt(a)))
end

--- Initial great-circle bearing from (lat1, lon1) toward (lat2, lon2),
--- quantised to one of 8 compass labels (N, NE, E, SE, S, SW, W, NW).
--- Returns nil when the two points are effectively coincident — i.e.
--- there is no meaningful direction.
function M.bearing_label(lat1, lon1, lat2, lon2)
    local rlat1 = deg2rad(lat1)
    local rlat2 = deg2rad(lat2)
    local dlon = lon2 - lon1
    if dlon > 180 then dlon = dlon - 360
    elseif dlon < -180 then dlon = dlon + 360 end
    dlon = deg2rad(dlon)
    local y = math.sin(dlon) * math.cos(rlat2)
    local x = math.cos(rlat1) * math.sin(rlat2)
        - math.sin(rlat1) * math.cos(rlat2) * math.cos(dlon)
    if math.abs(x) < 1e-12 and math.abs(y) < 1e-12 then return nil end
    local deg = (math.atan(y, x) * 180 / math.pi + 360) % 360
    local idx = math.floor((deg + 22.5) / 45) % 8
    return COMPASS[idx + 1]
end

--- Antipode of (lat, lon) — the diametrically opposite point on the
--- sphere. Returns lon in [-180, 180] regardless of input sign.
function M.antipode(lat, lon)
    local alon = lon + 180
    if alon > 180 then alon = alon - 360
    elseif alon < -180 then alon = alon + 360 end
    return -lat, alon
end

return M
