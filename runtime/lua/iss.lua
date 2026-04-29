-- iss (Lua port) — single moving marker for the International
-- Space Station.
--
-- Source: Open Notify
--   http://api.open-notify.org/iss-now.json
-- Free, no key. HTTP-only because the HTTPS alternative
-- (api.wheretheiss.at) ships an old TLS stack that reqwest can't
-- handshake with on some networks. Same transport baseline as the
-- default tile source.
--
-- Cadence: a 5 s refresh produces visibly smooth motion (~38 km
-- between samples) without hammering the upstream.
--
-- Response shape:
--   { "iss_position": { "latitude": "57.70", "longitude": "-31.74" },
--     "timestamp": 1620000000, "message": "success" }
-- Note: latitude / longitude come back as *strings*, not numbers.

local URL = "http://api.open-notify.org/iss-now.json"
local INTERVAL_SEC = 5
local ALTITUDE_KM = 408
local VELOCITY_KMS = 7.66

local state = {
    position = nil,        -- { lat, lon } once we've heard back
    last_update_sec = nil, -- os.time() of the latest successful fetch
    job = nil,             -- pending fetch
    last_fetch_sec = 0,
    initial_jump_done = false,
}

local function parse_position(payload)
    if not payload then return nil end
    local p = payload.iss_position
    if not p then return nil end
    -- Open Notify returns coordinates as strings, so tonumber() to
    -- get f64s for the bridge. Either parse failure → nil → drop.
    local lat = tonumber(p.latitude)
    local lon = tonumber(p.longitude)
    if not (lat and lon) then return nil end
    return { lat = lat, lon = lon }
end

local function age_text()
    if not state.last_update_sec then return "awaiting data" end
    local secs = math.min(os.time() - state.last_update_sec, 999)
    return string.format("updated %ds ago", secs)
end

return {
    name = "iss",

    -- Compact 3-row panel at the top-left so it doesn't compete
    -- with wiki's right-side or aircraft's left-side full-height
    -- panel. Width matches the Rust panel's footprint.
    layout = { anchor = "top-left", width = 30, height = 4 },

    render = function()
        local pos_line
        if state.position then
            pos_line = string.format("%.2f°N, %.2f°E",
                state.position.lat, state.position.lon)
        else
            pos_line = "(no position yet)"
        end
        return {
            pos_line,
            string.format("%d km @ %.2f km/s", ALTITUDE_KM, VELOCITY_KMS),
            age_text(),
        }
    end,

    paint_on_map = function(map)
        if state.position then
            map:point(state.position.lon, state.position.lat, "◉", "accent_alt")
            map:label(state.position.lon, state.position.lat, " ISS", "accent_alt")
        end
    end,

    handle_event = function(key)
        -- Enter recentres on the cached position. Pre-fetch we
        -- silently swallow Enter so it doesn't leak to the base
        -- layer mid-load.
        if key.code == "Enter" then
            if state.position then
                host:jump(state.position.lon, state.position.lat)
            end
            return nil
        end
        -- Non-modal: defer pan / zoom / quit to the base layer.
        return { ignore = true }
    end,

    poll = function()
        if state.job then
            local body = state.job:try_take()
            if body then
                local pos = parse_position(host:parse_json(body))
                if pos then
                    state.position = pos
                    state.last_update_sec = os.time()
                    -- Auto-recentre on the station the first time
                    -- a position arrives, so the marker is
                    -- immediately visible after toggling on.
                    if not state.initial_jump_done then
                        state.initial_jump_done = true
                        host:jump(pos.lon, pos.lat)
                    end
                end
                state.job = nil
            end
        end
        local now = os.time()
        if not state.job and (now - state.last_fetch_sec) >= INTERVAL_SEC then
            state.last_fetch_sec = now
            state.job = host:fetch_url(URL)
        end
    end,
}
