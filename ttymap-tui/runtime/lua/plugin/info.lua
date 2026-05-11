-- info (Lua port) — top-right always-on chrome showing centre /
-- cursor / zoom / pan speed + bearing / solar time / distance from
-- the user's IP-located home / reverse-geocoded place name.
--
-- The reverse-geocode lookup hits Nominatim's free `/reverse`
-- endpoint with a 5 s throttle (the upstream asks callers to stay
-- under 1 req/s; 5 s is comfortably under). Throttle is plugin-side
-- because the rest of the painted readout updates every frame.
--
-- Pan speed and bearing are derived from the haversine distance and
-- initial bearing between the centre at the start of the current
-- 1 s window and the centre now. `os.time()` gives integer-second
-- resolution which is plenty for a glance — the display is read at
-- human cadence, not frame cadence.
--
-- Solar time is `UTC + lon / 15h`, normalised to 0–24h. Pure mean
-- solar time — no time zones, no DST, just where the sun is in the
-- sky at this longitude.

local fmt = require "ttymap.fmt"
local loc = require "ttymap.location"

local NOMINATIM_URL = "https://nominatim.openstreetmap.org/reverse"
local INTERVAL_SEC = 5
local EARTH_RADIUS_M = 6378137.0   -- WGS-84 equatorial radius, same as scalebar.lua
local COMPASS = { "N", "NE", "E", "SE", "S", "SW", "W", "NW" }

local state = {
    place_name = nil,
    job = nil,
    last_query = nil,        -- "lat,lon" string for the in-flight or last fetch
    last_fetch_sec = 0,
    -- Pan-speed / bearing bookkeeping. We anchor at one position per
    -- integer second; when the second rolls over, we recompute both
    -- off the same anchor pair (so the bearing reflects exactly the
    -- direction the second's speed was earned in).
    speed_anchor_lat = nil,
    speed_anchor_lon = nil,
    speed_anchor_sec = nil,
    speed_mps = 0,
    bearing = nil,           -- compass label N/NE/.../NW, nil when idle
}

-- Kick the geoip resolution once at plugin load so `from here:` has
-- a value to display by the time the user notices the row. Empty cb
-- — we don't care about the result here; `loc.cached()` picks it up
-- on subsequent ticks.
loc.get(function() end)

local function reverse_url(lat, lon)
    return string.format("%s?lat=%f&lon=%f&format=json&zoom=10",
        NOMINATIM_URL, lat, lon)
end

local function format_place(payload)
    if not payload then return nil end
    local addr = payload.address
    local city
    if addr then
        city = addr.city or addr.town or addr.village
    end
    local country = addr and addr.country
    if city and country then
        return city .. ", " .. country
    elseif country then
        return country
    elseif city then
        return city
    elseif payload.display_name then
        return payload.display_name
    end
    return nil
end

-- Great-circle distance in metres, with antimeridian-aware longitude
-- wrapping so a 179 → -179 pan reads as ~250 km (the short way round)
-- rather than 40 000 km (the long way round).
local function haversine_m(lat1, lon1, lat2, lon2)
    local rlat1 = lat1 * math.pi / 180
    local rlat2 = lat2 * math.pi / 180
    local dlat = rlat2 - rlat1
    local dlon = lon2 - lon1
    if dlon > 180 then dlon = dlon - 360
    elseif dlon < -180 then dlon = dlon + 360 end
    dlon = dlon * math.pi / 180
    local a = math.sin(dlat / 2) ^ 2
        + math.cos(rlat1) * math.cos(rlat2) * math.sin(dlon / 2) ^ 2
    return 2 * EARTH_RADIUS_M * math.asin(math.min(1, math.sqrt(a)))
end

local function format_speed(mps)
    if mps < 1 then return "idle" end
    if mps < 1000 then return string.format("%d m/s", math.floor(mps + 0.5)) end
    return string.format("%.1f km/s", mps / 1000)
end

-- Initial great-circle bearing from (lat1, lon1) to (lat2, lon2),
-- quantised to one of 8 compass labels. Returns nil when the two
-- points are effectively coincident — speeds below the idle threshold
-- have no meaningful direction.
local function bearing_label(lat1, lon1, lat2, lon2)
    local rlat1 = lat1 * math.pi / 180
    local rlat2 = lat2 * math.pi / 180
    local dlon = lon2 - lon1
    if dlon > 180 then dlon = dlon - 360
    elseif dlon < -180 then dlon = dlon + 360 end
    dlon = dlon * math.pi / 180
    local y = math.sin(dlon) * math.cos(rlat2)
    local x = math.cos(rlat1) * math.sin(rlat2)
        - math.sin(rlat1) * math.cos(rlat2) * math.cos(dlon)
    if math.abs(x) < 1e-12 and math.abs(y) < 1e-12 then return nil end
    local deg = (math.atan(y, x) * 180 / math.pi + 360) % 360
    local idx = math.floor((deg + 22.5) / 45) % 8
    return COMPASS[idx + 1]
end

-- Mean solar time at `lon` derived from the system UTC clock —
-- 15° east of Greenwich is +1h, antimeridian is ±12h, normalised to
-- the 0–24h range. No DST, no time zones — this is "where is the
-- sun" not "what time is it on someone's wristwatch".
local function solar_time(lon)
    local t = os.date("!*t", os.time())
    local utc_h = t.hour + t.min / 60 + t.sec / 3600
    local h = (utc_h + lon / 15) % 24
    if h < 0 then h = h + 24 end
    local hh = math.floor(h)
    local mm = math.floor((h - hh) * 60 + 0.5)
    if mm == 60 then mm = 0; hh = (hh + 1) % 24 end
    return string.format("%02d:%02d", hh, mm)
end

-- Update state.speed_mps + state.bearing once per integer second
-- based on the haversine distance and bearing between the second's
-- anchor and the current centre. Within a second the displayed values
-- stay put — fine for glanceable UI and avoids divide-by-zero on
-- sub-second deltas.
local function tick_speed(lat, lon)
    local now = os.time()
    if state.speed_anchor_sec == nil then
        state.speed_anchor_lat = lat
        state.speed_anchor_lon = lon
        state.speed_anchor_sec = now
        return
    end
    local dt = now - state.speed_anchor_sec
    if dt <= 0 then return end
    local d = haversine_m(state.speed_anchor_lat, state.speed_anchor_lon, lat, lon)
    state.speed_mps = d / dt
    -- Bearing only meaningful when there's a real displacement.
    if d >= 1 then
        state.bearing = bearing_label(
            state.speed_anchor_lat, state.speed_anchor_lon, lat, lon)
    else
        state.bearing = nil
    end
    state.speed_anchor_lat = lat
    state.speed_anchor_lon = lon
    state.speed_anchor_sec = now
end

local function refresh(lat, lon)
    -- Always-running plugin; only kick a new fetch when the throttle
    -- window has elapsed and no other request is in flight.
    if state.job then return end
    local now = os.time()
    if (now - state.last_fetch_sec) < INTERVAL_SEC then return end
    state.last_fetch_sec = now
    state.last_query = string.format("%.4f,%.4f", lat, lon)
    state.job = ttymap.http:fetch(reverse_url(lat, lon))
end

ttymap.api.frame.on_tick(function(map)
    -- Drain the in-flight reverse-geocode job, if any.
    if state.job then
        local body = state.job:try_take()
        if body then
            local payload = ttymap.json:parse(body)
            state.place_name = format_place(payload)
            state.job = nil
        end
    end

    local lon, lat = map:center()
    local zoom = map:zoom()

    -- Kick a new fetch (subject to the per-plugin throttle).
    refresh(lat, lon)
    tick_speed(lat, lon)

    -- Layout grouped by topic, blank rows left as visual separators:
    --   rows 0-2   position  (center / cursor / zoom)
    --   row  4     motion    (speed + bearing)
    --   rows 6-7   environment (solar time / place name)
    --   row  9     relation  (distance from user's home)
    map:text_anchored("top-right", 0,
        string.format(" center: %.3f, %.3f ", lat, lon), "accent")

    local clon, clat = map:cursor()
    local cursor_line
    if clon and clat then
        cursor_line = string.format(" cursor: %.3f, %.3f ", clat, clon)
    else
        cursor_line = " cursor: unknown "
    end
    map:text_anchored("top-right", 1, cursor_line, "accent")

    map:text_anchored("top-right", 2,
        string.format(" zoom: %.1f ", zoom), "accent")

    local speed_line
    if state.bearing then
        speed_line = string.format(" speed: %s %s ",
            format_speed(state.speed_mps), state.bearing)
    else
        speed_line = string.format(" speed: %s ", format_speed(state.speed_mps))
    end
    map:text_anchored("top-right", 4, speed_line, "accent")

    map:text_anchored("top-right", 6,
        string.format(" solar: %s ", solar_time(lon)), "accent")

    local place = state.place_name or "unknown"
    map:text_anchored("top-right", 7,
        " place: " .. place .. " ", "accent")

    -- "from here" — distance from the user's IP-located home to the
    -- current centre. `loc.cached()` returns nil until the initial
    -- geoip lookup lands, in which case we show "unknown".
    local hlat, hlon = loc.cached()
    local from_here_line
    if hlat and hlon then
        local d = haversine_m(hlat, hlon, lat, lon)
        from_here_line = " from here: " .. fmt.distance(d) .. " "
    else
        from_here_line = " from here: unknown "
    end
    map:text_anchored("top-right", 9, from_here_line, "accent")
end)
