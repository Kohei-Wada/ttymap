-- aircraft (Lua port) — live ADS-B markers + side panel.
--
-- Top-level orchestrator: state, lifecycle (open/close/toggle),
-- per-frame `on_tick` driving fetch + paint, palette command. The
-- OpenSky REST client lives in `aircraft.opensky`; the formatting +
-- marker helpers live in `aircraft.display`.

local opensky = require("aircraft.opensky")
local display = require("aircraft.display")
local sidebar = require("ttymap.sidebar")

local state = {
    aircraft       = {},  -- list of { callsign, lon, lat, on_ground, alt, heading }
    selected       = 1,   -- 1-based index
    job            = nil, -- pending fetch
    last_fetch_sec = 0,   -- wall-clock second of last fetch start
    initial_done   = false, -- whether the first fetch after open landed
}
local w = nil  -- card handle while open; nil while closed (also acts as enabled flag)

-- Empty-state placeholder. Used by the bridge when `items()`
-- below returns an empty list (= no fetch result yet).
local function build_lines()
    return {
        { { text = "Loading...",           style = "muted" } },
        { { text = "(OpenSky takes ~12s)", style = "muted" } },
    }
end

local function build_items()
    local items = {}
    for _, a in ipairs(state.aircraft) do
        -- Each aircraft is a 1-line item.
        table.insert(items, { display.fmt(a) })
    end
    return items
end

-- Per-frame work runs only while the panel is open: drains the
-- in-flight fetch, schedules the next one, and paints markers.
-- Closing the panel (`w = nil`) immediately stops fetching, which
-- preserves the legacy "no traffic when hidden" budget behavior.
ttymap.api.frame.on_tick(function(map)
    if not w then return end
    -- Drain any in-flight fetch.
    if state.job then
        local body = state.job:try_take()
        if body then
            local payload = ttymap.json:parse(body)
            if not payload then
                -- Short-circuit so we don't follow up the warn with
                -- a misleading "0 in view" info popup; the existing
                -- panel state stays untouched until a healthy fetch.
                ttymap.notify("aircraft: OpenSky response unparseable",
                              { level = "warn" })
                state.job = nil
                return
            end
            state.aircraft = opensky.parse(payload)
            if state.selected > #state.aircraft then
                state.selected = math.max(1, #state.aircraft)
            end
            -- One info popup on the first successful fetch after
            -- opening the panel; subsequent refreshes stay quiet so
            -- the corner doesn't get spammed every interval.
            if not state.initial_done then
                state.initial_done = true
                ttymap.notify(string.format(
                    "aircraft: %d in view", #state.aircraft
                ))
            end
            state.job = nil
        end
    end
    -- Schedule the next fetch when the interval has elapsed.
    local now = os.time()
    if not state.job and (now - state.last_fetch_sec) >= opensky.INTERVAL_SEC then
        state.last_fetch_sec = now
        local lon, lat = map:center()
        state.job = ttymap.http:fetch(opensky.url(lon, lat))
    end
    -- Markers.
    for i, a in ipairs(state.aircraft) do
        local color = (i == state.selected) and "accent_alt" or "accent"
        map:point(a.lon, a.lat, display.marker_for(a), color)
    end
end)

local function close()
    if w then
        w:close()
        w = nil
        -- Reset the first-fetch flag so the next open re-shows the
        -- "N in view" popup. Without this, reopening would silently
        -- reuse stale state.
        state.initial_done = false
    end
end

local function open()
    if w then return end
    w = ttymap.api.card.open({
        footer_hints = {
            { key = "C-n/C-p", label = "select" },
            { key = "Enter",   label = "jump" },
            { key = "q / Esc", label = "close" },
        },
        render   = build_lines,
        items    = build_items,
        selected = function() return state.selected end,
        handle_key = function(key)
            local n = #state.aircraft
            if sidebar.up_pressed(key) then
                state.selected = sidebar.cycle(state.selected, n, -1)
                return nil
            end
            if sidebar.down_pressed(key) then
                state.selected = sidebar.cycle(state.selected, n, 1)
                return nil
            end
            if key.code == "Enter" then
                local a = state.aircraft[state.selected]
                if a then ttymap.map:jump(a.lon, a.lat) end
                return nil
            end
            if sidebar.is_close_key(key) then
                close()
                return nil
            end
            -- Anything else (j/k, hjkl, +/-, …) passes through to
            -- the base layer so map pan / zoom keep working while
            -- the section is focused.
            return { ignore = true }
        end,
    })
end

local function toggle()
    if w then close() else open() end
end

ttymap.register_palette_command({
    label  = "Toggle aircraft",
    invoke = toggle,
})
