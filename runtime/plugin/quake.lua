-- quake (Lua port) — recent earthquakes from the USGS public feed.
--
-- Source: USGS magnitude 2.5+ in the past 24h (~40-60 events
-- worldwide on a normal day).
--   https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson
--
-- Cadence: USGS itself updates ≈1/min; 5 min keeps load polite
-- while picking up new events promptly.
--
-- Renders markers only — no panel, no input. M5+ events surface
-- with the alt accent glyph; below that they get a routine dot.
-- On first successful fetch the map auto-jumps to the highest-
-- magnitude quake so the user always lands somewhere meaningful.
--
-- No window: visibility is a plugin-internal `enabled` flag flipped
-- by the activation callbacks. The `loop` short-circuits when off,
-- which preserves the legacy "no fetch when hidden" budget.

local URL = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson"
local INTERVAL_SEC = 300
local NOTABLE_MAGNITUDE = 5.0

local enabled = false

local state = {
    quakes = {},
    job = nil,
    last_fetch_sec = 0,
    initial_jump_done = false,
}

local function parse_features(payload)
    local out = {}
    if not payload or not payload.features then return out end
    for _, f in ipairs(payload.features) do
        local geom = f.geometry
        local props = f.properties
        if geom and props then
            local coords = geom.coordinates
            if type(coords) == "table" then
                local lon = coords[1]
                local lat = coords[2]
                local mag = props.mag
                if type(lon) == "number" and type(lat) == "number"
                    and type(mag) == "number" then
                    table.insert(out, { lon = lon, lat = lat, mag = mag })
                end
            end
        end
    end
    return out
end

local function highest_magnitude(qs)
    local best
    for _, q in ipairs(qs) do
        if not best or q.mag > best.mag then best = q end
    end
    return best
end

-- Per-frame work runs only while the plugin is enabled: drains
-- the in-flight fetch, schedules the next, and paints markers.
-- Toggling off (`enabled = false`) immediately stops fetching.
ttymap.api.frame.on_tick(function(map)
    if not enabled then return end
    if state.job then
        local body = state.job:try_take()
        if body then
            local payload = ttymap.json:parse(body)
            state.quakes = parse_features(payload)
            -- Auto-recentre on the first non-empty result so
            -- the user lands somewhere meaningful right after
            -- toggling on.
            if not state.initial_jump_done and #state.quakes > 0 then
                local top = highest_magnitude(state.quakes)
                if top then
                    state.initial_jump_done = true
                    ttymap.map:jump(top.lon, top.lat)
                end
            end
            state.job = nil
        end
    end
    local now = os.time()
    if not state.job and (now - state.last_fetch_sec) >= INTERVAL_SEC then
        state.last_fetch_sec = now
        state.job = ttymap.http:fetch(URL)
    end
    for _, q in ipairs(state.quakes) do
        if q.mag >= NOTABLE_MAGNITUDE then
            map:point(q.lon, q.lat, "✸", "accent_alt")
        else
            map:point(q.lon, q.lat, "·", "accent")
        end
    end
end)

local function toggle()
    enabled = not enabled
end

ttymap.register_palette_command({
    label = "Toggle quakes",
    invoke = toggle,
})
