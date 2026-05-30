-- aircraft (Lua port) — live ADS-B markers + side panel.
--
-- Top-level orchestrator: state, lifecycle (open/close/toggle),
-- per-frame `on_tick` driving fetch + paint, palette command. The
-- OpenSky REST client lives in `aircraft.opensky`; the formatting +
-- marker helpers live in `aircraft.display`.

local opensky = require("plugin.aircraft.opensky")
local display = require("plugin.aircraft.display")
local sidebar = require("ttymap.sidebar")
local anim    = require("ttymap.animation")

local state = {
    aircraft       = {},  -- list of { callsign, lon, lat, on_ground, alt, ... }
    selected       = 1,   -- 1-based index
    job            = nil, -- pending fetch
    last_fetch_sec = 0,   -- wall-clock second of last fetch start
    initial_done   = false, -- whether the first fetch after open landed
    selected_icao  = nil, -- icao24 of the selected plane (pins selection
                          -- across refreshes; index alone points elsewhere)
    cur_col        = nil, -- last cursor cell (for hover-select movement gate)
    cur_row        = nil,
}
local w = nil       -- card handle while open; nil while closed
local tick_handle = nil  -- on_tick subscription while open; nil while closed

-- Empty-state placeholder. Used by the bridge when `items()`
-- below returns an empty list (= no fetch result yet).
local function build_lines()
    return {
        { { text = "Loading...",          style = "muted" } },
        { { text = "(fetching OpenSky…)", style = "muted" } },
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

-- Centre the map on the currently selected aircraft. Called whenever
-- the selection changes (keyboard nav or hover) so the view follows
-- the list, wiki-plugin style.
local function fly_to_selected()
    local a = state.aircraft[state.selected]
    if a then
        state.selected_icao = a.icao
        anim.fly_to(a.lon, a.lat)
    end
end

-- Per-frame work: drain the in-flight fetch, schedule the next one,
-- and paint markers. Subscribed only while the panel is open — when
-- the panel closes the on_tick handle is `:remove()`d so this
-- callback is gone from the bus iteration entirely (no per-frame
-- fetch / paint cost when aircraft is hidden).
local function on_tick(map)
    -- Advance the OpenSky OAuth token (no-op when unconfigured).
    opensky.poll_auth()
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
            local clon, clat = map:center()
            state.aircraft = opensky.limit_to_center(
                opensky.parse(payload), clon, clat
            )
            -- Re-pin the selection to the same physical plane (by
            -- icao) in the refreshed list, so the highlight doesn't
            -- jump to whatever now sits at the old index.
            if state.selected_icao then
                for i, a in ipairs(state.aircraft) do
                    if a.icao == state.selected_icao then
                        state.selected = i
                        break
                    end
                end
            end
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
    if not state.job and (now - state.last_fetch_sec) >= opensky.interval_sec() then
        state.last_fetch_sec = now
        local lon, lat = map:center()
        state.job = opensky.fetch_states(lon, lat)
    end
    -- Hover-select: when the mouse moves onto (within ~2 cells of) a
    -- marker, select that aircraft. Gated on cursor *movement* so
    -- keyboard selection (C-n/C-p) still works while the mouse is
    -- parked; leaves the selection untouched when the cursor is over
    -- empty space.
    local clon, clat = map:cursor()
    if clon then
        local ccol, crow = map:project(clon, clat)
        if ccol and (ccol ~= state.cur_col or crow ~= state.cur_row) then
            state.cur_col, state.cur_row = ccol, crow
            local best, best_d = nil, 3   -- chebyshev distance < 3 ⇒ within 2 cells
            for i, a in ipairs(state.aircraft) do
                local acol, arow = map:project(a.lon, a.lat)
                if acol then
                    local d = math.max(math.abs(acol - ccol), math.abs(arow - crow))
                    if d < best_d then best, best_d = i, d end
                end
            end
            if best and best ~= state.selected then
                state.selected = best
                fly_to_selected()
            end
        end
    end
    -- Markers.
    for i, a in ipairs(state.aircraft) do
        local color = (i == state.selected) and "accent_alt" or "accent"
        map:point(a.lon, a.lat, display.marker_for(a), color)
    end
end

local function close()
    if w then
        w:close()
        w = nil
        if tick_handle then
            tick_handle:remove()
            tick_handle = nil
        end
        -- Reset the first-fetch flag so the next open re-shows the
        -- "N in view" popup. Without this, reopening would silently
        -- reuse stale state.
        state.initial_done = false
    end
end

local function open()
    if w then return end
    tick_handle = ttymap.api.frame.on_tick(on_tick)
    w = ttymap.api.card.open({
        name = "aircraft",
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
                fly_to_selected()
                return nil
            end
            if sidebar.down_pressed(key) then
                state.selected = sidebar.cycle(state.selected, n, 1)
                fly_to_selected()
                return nil
            end
            if key.code == "Enter" then
                fly_to_selected()
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
