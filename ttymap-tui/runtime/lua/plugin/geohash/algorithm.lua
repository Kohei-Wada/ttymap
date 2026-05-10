-- algorithm.lua — xkcd #426 geohashing coordinate computation.
--
-- The algorithm (https://geohashing.site/geohashing/The_Algorithm):
--
--   1. Concatenate today's date (YYYY-MM-DD) with the Dow Jones
--      opening value as "YYYY-MM-DD-DDDDD.DD".
--   2. MD5 the seed → 32 lowercase hex chars.
--   3. Split in half. Each half (16 hex chars) is a base-16 fraction
--      in [0, 1).
--   4. Add the fractions to the integer part of the user's lat / lon
--      graticule. For NEGATIVE graticules, SUBTRACT instead so the
--      point lands inside the same graticule rather than crossing
--      into the next-toward-zero one.
--
-- See `init.lua` for the 30W rule (which DJIA date to use west of
-- -30° longitude) — that's a wrapper concern, not part of the core
-- coordinate math.

local md5 = require "plugin.geohash.md5"

local M = {}

-- Convert 16 hex chars to a base-16 fraction in [0, 1).
local function hex_to_fraction(hex)
    local f = 0
    for i = 1, #hex do
        f = f + tonumber(hex:sub(i, i), 16) * (16 ^ -i)
    end
    return f
end

-- Compute the geohash target inside the (lat_int, lon_int) graticule
-- for the given (date, djia) pair.
--   date   — "YYYY-MM-DD" string
--   djia   — string like "42158.22" (kept as a string so the seed
--            preserves the upstream's exact decimal formatting; the
--            algorithm hashes the literal characters)
--   lat_int, lon_int — graticule indices (truncate-toward-zero of
--                      lat/lon; e.g. lat -33.5 → -33, NOT -34)
-- Returns target_lat, target_lon.
function M.compute(date, djia, lat_int, lon_int)
    local seed = string.format("%s-%s", date, djia)
    local hash = md5.hex(seed)
    local lat_frac = hex_to_fraction(hash:sub(1, 16))
    local lon_frac = hex_to_fraction(hash:sub(17, 32))
    local target_lat = lat_int + (lat_int < 0 and -lat_frac or lat_frac)
    local target_lon = lon_int + (lon_int < 0 and -lon_frac or lon_frac)
    return target_lat, target_lon
end

-- Truncate-toward-zero. The geohashing graticule for -33.5 is -33,
-- not floor(-33.5) = -34: a graticule is "the 1°×1° square whose
-- corner closest to the equator/prime-meridian is at this integer."
function M.graticule_of(coord)
    local int = math.modf(coord)
    return int
end

-- Great-circle distance in km, Haversine. Used only for the
-- "X km from you" sidebar readout — surveyor accuracy is not needed.
function M.haversine_km(lat1, lon1, lat2, lon2)
    local R = 6371.0
    local rad = math.pi / 180
    local dlat = (lat2 - lat1) * rad
    local dlon = (lon2 - lon1) * rad
    local a = math.sin(dlat / 2) ^ 2
        + math.cos(lat1 * rad) * math.cos(lat2 * rad)
            * math.sin(dlon / 2) ^ 2
    return 2 * R * math.asin(math.sqrt(a))
end

-- 8-sector compass bearing from (lat1, lon1) toward (lat2, lon2).
-- Same purpose as haversine_km — sidebar readout only, no claim of
-- precision past one of N / NE / E / SE / S / SW / W / NW.
function M.bearing_8(lat1, lon1, lat2, lon2)
    local rad = math.pi / 180
    local y = math.sin((lon2 - lon1) * rad) * math.cos(lat2 * rad)
    local x = math.cos(lat1 * rad) * math.sin(lat2 * rad)
        - math.sin(lat1 * rad) * math.cos(lat2 * rad)
            * math.cos((lon2 - lon1) * rad)
    local deg = (math.atan(y, x) / rad + 360) % 360
    local sectors = { "N", "NE", "E", "SE", "S", "SW", "W", "NW" }
    local idx = math.floor((deg + 22.5) / 45) % 8 + 1
    return sectors[idx]
end

return M
