-- quake (Lua port) — recent earthquakes from the USGS public feed.
--
-- Source: USGS magnitude 2.5+ in the past 24h (~40-60 events
-- worldwide on a normal day).
--   https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson
--
-- Cadence: USGS itself updates ≈1/min; 5 min keeps load polite
-- while picking up new events promptly.
--
-- Panel: sidebar list of quakes sorted by magnitude (highest first),
-- highlighted row jumps to that quake on Enter. Markers + map paint
-- continue while the sidebar is closed (toggling the panel only
-- hides the panel; pause-when-hidden lives behind the palette
-- "Toggle quakes" command instead).
--
-- M5+ events surface with the alt accent glyph; below that they get
-- a routine dot. On first successful fetch the map auto-jumps to
-- the highest-magnitude quake so the user always lands somewhere
-- meaningful.

local URL = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_day.geojson"
local INTERVAL_SEC = 300
local NOTABLE_MAGNITUDE = 5.0

local enabled = false  -- "feed running"; flipped by palette command
local w = nil          -- sidebar handle while open

local state = {
    quakes = {},   -- list of { lon, lat, mag, place, time_ms }
    selected = 1,  -- 1-based index into quakes
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
                    table.insert(out, {
                        lon = lon,
                        lat = lat,
                        mag = mag,
                        place = type(props.place) == "string" and props.place or "",
                        time_ms = type(props.time) == "number" and props.time or 0,
                    })
                end
            end
        end
    end
    -- Highest magnitude first, so notable events float to the top
    -- of the panel and the auto-jump lands somewhere meaningful.
    table.sort(out, function(a, b) return a.mag > b.mag end)
    return out
end

local function highest_magnitude(qs)
    -- After the sort above the top entry already wins; kept as a
    -- function for clarity at the call site.
    return qs[1]
end

-- Format milliseconds-since-epoch as a coarse "Nh ago" / "Nm ago" /
-- "Nd ago" string. Quake feed updates ~once per minute, so per-second
-- precision adds churn without information.
local function ago(now_sec, ms)
    if not ms or ms <= 0 then return "" end
    local secs = now_sec - math.floor(ms / 1000)
    if secs < 0 then secs = 0 end
    if secs < 60 then return secs .. "s ago" end
    local mins = math.floor(secs / 60)
    if mins < 60 then return mins .. "m ago" end
    local hours = math.floor(mins / 60)
    if hours < 24 then return hours .. "h ago" end
    return math.floor(hours / 24) .. "d ago"
end

local function build_lines()
    if not enabled then
        return {
            { { text = "(feed off)",                     style = "muted" } },
            { { text = "Toggle from :Toggle quakes",     style = "muted" } },
        }
    end
    if #state.quakes == 0 then
        return {
            { { text = "Loading...",      style = "muted" } },
            { { text = "(USGS, 2.5+/24h)", style = "muted" } },
        }
    end
    local now_sec = os.time()
    local lines = {}
    for i, q in ipairs(state.quakes) do
        local title_style = (i == state.selected) and "highlight" or "accent"
        local mag_str = string.format("M%.1f", q.mag)
        local place = q.place ~= "" and q.place or "(unknown location)"
        table.insert(lines, {
            { text = mag_str,           style = title_style },
            { text = "  " .. place,     style = "body" },
        })
        local secondary = ago(now_sec, q.time_ms)
        if secondary ~= "" then
            table.insert(lines, {
                { text = "  " .. secondary, style = "muted" },
            })
        end
    end
    return lines
end

-- Per-frame work: drains the in-flight fetch, schedules the next
-- one, and paints markers. Driven by the `enabled` flag (palette
-- toggle) — the sidebar visibility is independent.
ttymap.api.frame.on_tick(function(map)
    if not enabled then return end
    if state.job then
        local body = state.job:try_take()
        if body then
            local payload = ttymap.json:parse(body)
            state.quakes = parse_features(payload)
            if state.selected > #state.quakes then
                state.selected = math.max(1, #state.quakes)
            end
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
    for i, q in ipairs(state.quakes) do
        local color
        if i == state.selected and w then
            color = "accent_alt"
        elseif q.mag >= NOTABLE_MAGNITUDE then
            color = "accent_alt"
        else
            color = "accent"
        end
        local marker = q.mag >= NOTABLE_MAGNITUDE and "✸" or "·"
        map:point(q.lon, q.lat, marker, color)
    end
end)

local function close_panel()
    if w then
        w:close()
        w = nil
    end
end

local function open_panel()
    if w then return end
    w = ttymap.api.window.open({
        name = "quake",
        layout = { kind = "sidebar" },
        footer_hints = {
            { key = "C-n/C-p", label = "select" },
            { key = "Enter",   label = "jump" },
            { key = "q / Esc", label = "close" },
        },
        render = build_lines,
        handle_event = function(key)
            local code = key.code
            local ch = key.char
            local ctrl = key.ctrl

            local up   = (ctrl and code == "Char" and ch == "p") or code == "Up"
            local down = (ctrl and code == "Char" and ch == "n") or code == "Down"

            local n = #state.quakes
            if up then
                if n > 0 then
                    state.selected = state.selected > 1 and state.selected - 1 or n
                end
                return nil
            end
            if down then
                if n > 0 then
                    state.selected = state.selected < n and state.selected + 1 or 1
                end
                return nil
            end
            if code == "Enter" then
                local q = state.quakes[state.selected]
                if q then ttymap.map:jump(q.lon, q.lat) end
                return nil
            end
            if code == "Esc" or (code == "Char" and ch == "q" and not ctrl) then
                close_panel()
                return nil
            end
            -- Anything else (j/k, q, hjkl, +/-, …) passes through
            -- to the base layer so map pan / zoom / quit keep
            -- working while the section is focused.
            return { ignore = true }
        end,
    })
end

local function toggle_feed()
    enabled = not enabled
end

local function toggle_panel()
    if w then close_panel() else open_panel() end
end

ttymap.register_palette_command({
    label = "Toggle quakes",
    invoke = toggle_feed,
})

ttymap.register_palette_command({
    label = "Show quake panel",
    invoke = toggle_panel,
})
