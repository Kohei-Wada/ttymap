-- aircraft (Lua port) — live ADS-B markers + side panel.
--
-- Top-level orchestrator: state, lifecycle (open/close/toggle),
-- per-frame `on_tick` driving fetch + paint, palette command. The
-- OpenSky REST client lives in `aircraft.opensky`; the formatting +
-- marker helpers live in `aircraft.display`.

local opensky = require("aircraft.opensky")
local display = require("aircraft.display")

local state = {
    aircraft       = {},  -- list of { callsign, lon, lat, on_ground, alt, heading }
    selected       = 1,   -- 1-based index
    job            = nil, -- pending fetch
    last_fetch_sec = 0,   -- wall-clock second of last fetch start
}
local w = nil  -- window handle while open; nil while closed (also acts as enabled flag)

local function build_lines()
    if #state.aircraft == 0 then
        return {
            { { text = "Loading...",            style = "muted" } },
            { { text = "(OpenSky takes ~12s)", style = "muted" } },
        }
    end
    local lines = {}
    for i, a in ipairs(state.aircraft) do
        table.insert(lines, display.fmt(a, i == state.selected))
    end
    return lines
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
            state.aircraft = opensky.parse(payload)
            if state.selected > #state.aircraft then
                state.selected = math.max(1, #state.aircraft)
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
    end
end

local function open()
    if w then return end
    w = ttymap.api.window.open({
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

            local n = #state.aircraft
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
                local a = state.aircraft[state.selected]
                if a then ttymap.map:jump(a.lon, a.lat) end
                return nil
            end
            if code == "Esc" or (code == "Char" and ch == "q" and not ctrl) then
                close()
                return nil
            end
            -- Anything else (j/k, q, hjkl, +/-, …) passes through to
            -- the base layer so map pan / zoom / quit keep working
            -- while the section is focused.
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
