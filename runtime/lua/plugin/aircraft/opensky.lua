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

local cfg = require("ttymap.aircraft")

local M = {}

local BASE_URL       = "https://opensky-network.org/api/states/all"
local TOKEN_URL      =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token"
local BBOX_HALF_DEG  = 5.0   -- half-side of the bbox sent per fetch

-- OAuth2 client-credentials state (see ttymap.aircraft). Anonymous
-- when credentials are unset; otherwise we hold a Bearer token and
-- refresh it ~1 min before its `expires_in` (≈30 min) lapses.
local auth = {
    token      = nil,
    expires_at = 0,    -- os.time() past which `token` is treated as stale
    job        = nil,  -- in-flight token POST
    notified   = false,-- one-shot "authenticated" toast (per program run)
}

-- Refresh cadence. OpenSky's state resolution is 5 s authenticated /
-- 10 s anonymous, so polling faster just re-fetches identical data.
-- Authenticated → 5 s, anonymous → 12 s; `ttymap.aircraft.interval_sec`
-- overrides. (At ~2 credits/call, 5 s burns ~24/min, so the 4000/day
-- authed budget lasts ~2.5 h of continuous viewing.)
function M.interval_sec()
    if cfg.interval_sec then return cfg.interval_sec end
    if auth.token then return 5 end
    return 12
end

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

-- True when OAuth2 credentials are configured (read lazily so a
-- value set in init.lua after this module loads still counts).
local function configured()
    return cfg.client_id ~= nil and cfg.client_secret ~= nil
end

-- Advance the token state machine. Call once per tick. No-op when
-- unconfigured (anonymous). Drains an in-flight token POST, then kicks
-- a refresh when the current token is missing/stale and none is in
-- flight. On failure it backs off ~60s so bad credentials don't hammer
-- the token endpoint every frame.
function M.poll_auth()
    if not configured() then return end

    if auth.job then
        local body = auth.job:try_take()
        if body then
            auth.job = nil
            local payload = ttymap.json:parse(body)
            if payload and payload.access_token then
                auth.token = payload.access_token
                local ttl = tonumber(payload.expires_in) or 1800
                auth.expires_at = os.time() + ttl - 60
                -- Tell the user which source is live, once per run.
                if not auth.notified then
                    auth.notified = true
                    ttymap.notify("aircraft: reading via OpenSky API (authenticated)")
                end
            else
                auth.token = nil
                auth.expires_at = os.time() + 60
                ttymap.notify(
                    "aircraft: OpenSky token fetch failed (check credentials)",
                    { level = "warn" }
                )
            end
        end
    end

    if not auth.job and os.time() >= auth.expires_at then
        auth.job = ttymap.http:fetch(TOKEN_URL, {
            method = "POST",
            form   = {
                grant_type    = "client_credentials",
                client_id     = cfg.client_id,
                client_secret = cfg.client_secret,
            },
        })
    end
end

-- Fetch state vectors around (lon, lat). Adds the Bearer header once a
-- token is in hand; falls back to an anonymous GET while unconfigured
-- or before the first token lands.
function M.fetch_states(lon, lat)
    if configured() and auth.token then
        return ttymap.http:fetch(M.url(lon, lat), {
            headers = { Authorization = "Bearer " .. auth.token },
        })
    end
    return ttymap.http:fetch(M.url(lon, lat))
end

-- Build the display list: cap to the nearest `ttymap.aircraft.max_count`
-- aircraft to the map centre `(lon, lat)` (equirectangular ranking,
-- longitude scaled by cos(lat)), then sort by icao24 so the row order
-- is *stable* across refreshes — the same plane keeps its row instead
-- of the whole table reshuffling every fetch.
function M.limit_to_center(list, lon, lat)
    local max = cfg.max_count
    if max and #list > max then
        local cos_lat = math.cos(math.rad(lat))
        local function d2(a)
            local dlon = a.lon - lon
            if dlon > 180 then dlon = dlon - 360 elseif dlon < -180 then dlon = dlon + 360 end
            dlon = dlon * cos_lat
            local dlat = a.lat - lat
            return dlon * dlon + dlat * dlat
        end
        table.sort(list, function(a, b) return d2(a) < d2(b) end)
        local nearest = {}
        for i = 1, max do nearest[i] = list[i] end
        list = nearest
    end
    table.sort(list, function(a, b) return (a.icao or "") < (b.icao or "") end)
    return list
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
                icao      = s[1],          -- icao24 hex id (stable key)
                callsign  = trim(s[2]),
                country   = trim(s[3]),
                lon       = lon,
                lat       = lat,
                on_ground = s[9] == true,
                alt       = s[8],          -- baro_altitude (m)
                velocity  = s[10],         -- ground speed (m/s)
                heading   = s[11],         -- true_track (deg)
                vrate     = s[12],         -- vertical_rate (m/s, +up)
                squawk    = s[15],
            })
        end
    end
    return out
end

return M
