-- aircraft (Lua port) — live ADS-B markers + side panel.
--
-- Proof-of-concept port of `src/plugin/aircraft/` to validate the
-- Lua bridge end-to-end. Opt-in via [lua_aircraft] enabled = true
-- so it can run side by side with the Rust plugin during the
-- transition.
--
-- Source: OpenSky Network REST API
--   https://opensky-network.org/api/states/all
-- Anonymous; the call costs 4 credits without a bbox. We refresh
-- once every 12 seconds, same cadence as the Rust plugin.
--
-- State-vector indices follow the OpenSky doc:
--   1 = icao24, 2 = callsign, 6 = lon, 7 = lat,
--   8 = baro_altitude, 9 = on_ground, 10 = velocity, 11 = true_track
-- (Lua arrays are 1-indexed; OpenSky's docs are 0-indexed.)

local URL = "https://opensky-network.org/api/states/all"
local INTERVAL_SEC = 12

local state = {
    aircraft = {},      -- list of { callsign, lon, lat, on_ground, alt }
    selected = 1,       -- 1-based index
    job = nil,          -- pending fetch
    last_fetch_sec = 0, -- wall-clock second of last fetch start
}

local function trim(s)
    return (s or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function parse_states(payload)
    local out = {}
    if not payload or not payload.states then return out end
    for _, s in ipairs(payload.states) do
        -- s is the state-vector array. Skip rows missing lon/lat
        -- (fresh tracks before first position lock).
        local lon, lat = s[6], s[7]
        if type(lon) == "number" and type(lat) == "number" then
            table.insert(out, {
                callsign  = trim(s[2]),
                lon       = lon,
                lat       = lat,
                on_ground = s[9] == true,
                alt       = s[8],
            })
        end
    end
    return out
end

local function fmt_aircraft(a, selected)
    local prefix = selected and "→ " or "  "
    local cs     = a.callsign ~= "" and a.callsign or "(no callsign)"
    local alt    = ""
    if type(a.alt) == "number" then
        alt = string.format(" %dm", math.floor(a.alt))
    end
    local ground = a.on_ground and " (ground)" or ""
    return prefix .. cs .. alt .. ground
end

return {
    name = "aircraft",

    -- Mirror the Rust plugin's default placement: left-side stripe,
    -- 40 cells wide, full available height.
    layout = { anchor = "left", width = 40 },

    render = function()
        if #state.aircraft == 0 then
            return { "Loading aircraft data...", "(OpenSky takes ~12s)" }
        end
        local lines = {}
        for i, a in ipairs(state.aircraft) do
            table.insert(lines, fmt_aircraft(a, i == state.selected))
        end
        return lines
    end,

    paint_on_map = function(map)
        for i, a in ipairs(state.aircraft) do
            local color = (i == state.selected) and "accent_alt" or "accent"
            map:point(a.lon, a.lat, "✈", color)
        end
    end,

    handle_event = function(key)
        if key.code == "Esc" then return { close = true } end
        if key.code == "Char" and key.char == "q" then return { close = true } end

        local n = #state.aircraft
        if key.code == "Up" or (key.code == "Char" and key.char == "k") then
            if n > 0 then
                state.selected = state.selected > 1 and state.selected - 1 or n
            end
        elseif key.code == "Down" or (key.code == "Char" and key.char == "j") then
            if n > 0 then
                state.selected = state.selected < n and state.selected + 1 or 1
            end
        elseif key.code == "Enter" then
            local a = state.aircraft[state.selected]
            if a then host:jump(a.lon, a.lat) end
        end
        -- Modal feel: consume otherwise.
        return nil
    end,

    poll = function()
        -- Drain any in-flight fetch.
        if state.job then
            local body = state.job:try_take()
            if body then
                local payload = host:parse_json(body)
                state.aircraft = parse_states(payload)
                if state.selected > #state.aircraft then
                    state.selected = math.max(1, #state.aircraft)
                end
                state.job = nil
            end
        end
        -- Schedule the next fetch when the interval has elapsed.
        local now = os.time()
        if not state.job and (now - state.last_fetch_sec) >= INTERVAL_SEC then
            state.last_fetch_sec = now
            state.job = host:fetch_url(URL)
        end
    end,
}
