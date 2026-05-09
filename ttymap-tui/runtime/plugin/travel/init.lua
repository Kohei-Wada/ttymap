-- travel — curated multi-country travel itineraries with an animated
-- tour mode that flies the camera through each stop.
--
-- Three sidebar states (transitions on Enter / Esc):
--
--   1. **list**: every bundled route across all countries. C-n / C-p
--      navigates, Enter opens detail.
--   2. **detail**: full itinerary for the selected route — country,
--      days, summary, stop-by-stop notes. Map preview shows the
--      whole polyline + all markers so the geographic shape of the
--      trip is visible at a glance. Enter starts the tour, Esc /
--      Backspace returns to list.
--   3. **tour**: the camera flies through three phases (pre overview
--      → stop loop → post overview) driven by `ttymap.director` —
--      the whole choreography is a single procedural Lua script.
--      `state.tour.phase` is set inline as the script runs so the
--      sidebar / on_tick paint can tell where we are. Manual pan /
--      zoom (or palette re-toggle) cancels at any point via the
--      animation lib's tolerance check → director.on_cancel.
--      No polyline during fly motion (Braille pixel-snapping
--      ripples a static line as the camera glides) — markers carry
--      the journey on their own; the line surfaces only during
--      the pre/post overview *dwells* where the camera is parked.
--
-- Activation: `:` palette → "Travel". Re-invoking always exits
-- completely (cancels any tour, closes the panel).
--
-- Adding a country: see `travel/routes/init.lua`.

local sidebar   = require("ttymap.sidebar")
local director  = require("ttymap.director")
local countries = require("travel.routes")

-- Flatten the per-country tables into a single list of routes,
-- annotating each with the country name so the sidebar can show
-- "Japan · Golden Route" without needing to maintain a parallel
-- index. Shallow-copy each route so we don't mutate the requires
-- cache (Lua caches requires; mutating would compound on reload).
local routes = {}
for _, c in ipairs(countries) do
    for _, r in ipairs(c.routes) do
        local annotated = {}
        for k, v in pairs(r) do annotated[k] = v end
        annotated.country = c.country
        table.insert(routes, annotated)
    end
end

-- Phase budgets (60fps assumption — frame counts, not ms, since
-- on_tick doesn't surface dt). Tight 2-second beats keep momentum:
-- long enough to read the notify popup and absorb the view, short
-- enough that a 5-stop tour finishes in ~15s.
local DWELL_FRAMES = 120

-- Compute a (centre, zoom) that fits all stops with margin. Logarithmic
-- zoom heuristic: span doubles → zoom drops by 1. Clamped to a sensible
-- range so single-city routes don't zoom in past street level and
-- continent-spanning ones don't blow past world view.
local function overview_view(route)
    local min_lon, max_lon = math.huge, -math.huge
    local min_lat, max_lat = math.huge, -math.huge
    for _, s in ipairs(route.stops) do
        if s.lon < min_lon then min_lon = s.lon end
        if s.lon > max_lon then max_lon = s.lon end
        if s.lat < min_lat then min_lat = s.lat end
        if s.lat > max_lat then max_lat = s.lat end
    end
    local center_lon = (min_lon + max_lon) / 2
    local center_lat = (min_lat + max_lat) / 2
    local span = math.max(max_lon - min_lon, max_lat - min_lat, 0.1)
    local zoom = math.floor(8.5 - math.log(span) / math.log(2))
    if zoom < 3 then zoom = 3 elseif zoom > 11 then zoom = 11 end
    return center_lon, center_lat, zoom
end

local state = {
    selected = 1,    -- 1-based index into routes (list mode)
    w        = nil,  -- sidebar card handle while open
    detail   = nil,  -- nil = list mode; a route table = detail mode
    tour     = nil,  -- nil when not playing; else { route, phase, stop_idx }
    handle   = nil,  -- director handle for the active tour
}
local tick_handle = nil  -- on_tick subscription while the panel is open

local function close_panel()
    if state.w then
        state.w:close()
        state.w = nil
        if tick_handle then
            tick_handle:remove()
            tick_handle = nil
        end
    end
end

local function start_tour(route)
    local lon, lat, z = overview_view(route)
    state.tour = { route = route, phase = "pre_fly", stop_idx = 0 }
    state.handle = director.run(function()
        ttymap.notify(string.format("Starting: %s · %s",
            route.country, route.name))
        director.fly(lon, lat, z)

        state.tour.phase = "pre_dwell"
        director.wait(DWELL_FRAMES)

        for i, stop in ipairs(route.stops) do
            state.tour.phase = "stop_fly"
            state.tour.stop_idx = i
            director.fly(stop.lon, stop.lat, stop.zoom)

            state.tour.phase = "stop_dwell"
            ttymap.notify(stop.note)
            director.wait(DWELL_FRAMES)
        end

        state.tour.phase = "post_fly"
        ttymap.notify(string.format("Tour complete: %s · %s",
            route.country, route.name))
        director.fly(lon, lat, z)

        state.tour.phase = "post_dwell"
        director.wait(DWELL_FRAMES)

        state.tour   = nil
        state.handle = nil
    end, {
        on_cancel = function()
            state.tour   = nil
            state.handle = nil
            ttymap.notify("Tour cancelled")
        end,
    })
end

local function cancel_tour_silent()
    if state.handle then
        state.handle:cancel({ silent = true })
        state.handle = nil
    end
    state.tour = nil
end

-- Per-frame map paint. Subscribed only while the panel is open; the
-- callback handles three cases in priority order:
--
--   * **tour active**: paint markers for every stop, coloured by
--     phase + position (current accent, past accent_alt, future
--     muted). The polyline surfaces only during pre/post overview
--     dwells (camera parked = stable line). Phase is set inline by
--     the director script in `start_tour`.
--   * **detail mode**: no tour, but a route is selected — paint the
--     whole polyline + all markers as a preview. Camera is static
--     here (the user hasn't started flying), so the line is stable.
--   * **list mode**: nothing to paint.
local function on_tick(map)
    local t = state.tour
    if t then
        local route = t.route
        local current_idx = (t.phase == "stop_fly" or t.phase == "stop_dwell")
            and t.stop_idx or 0

        if t.phase == "pre_dwell" or t.phase == "post_dwell" then
            local coords = {}
            for _, s in ipairs(route.stops) do
                table.insert(coords, { s.lon, s.lat })
            end
            map:polyline(coords, "muted")
        end

        for i, s in ipairs(route.stops) do
            local color
            if i == current_idx then
                color = "accent"
            elseif t.phase == "post_fly" or t.phase == "post_dwell"
                or i < current_idx then
                color = "accent_alt"
            else
                color = "muted"
            end
            map:point(s.lon, s.lat, "●", color)
            map:label(s.lon, s.lat, s.name, color)
        end
        return
    end

    local d = state.detail
    if d then
        local coords = {}
        for _, s in ipairs(d.stops) do
            table.insert(coords, { s.lon, s.lat })
        end
        map:polyline(coords, "muted")
        for _, s in ipairs(d.stops) do
            map:point(s.lon, s.lat, "●", "accent_alt")
        end
    end
end

-- ── Detail-mode rendering ─────────────────────────────────────────
local function phase_label_for(tour)
    local p = tour.phase
    if p == "pre_fly" or p == "pre_dwell" then
        return "Overview — orienting…"
    elseif p == "post_fly" or p == "post_dwell" then
        return "Wrap-up — overview…"
    else
        return "Tour playing — pan / zoom to cancel."
    end
end

local function render_detail(route)
    local lines = {
        { { text = route.country, style = "muted" },
          { text = " · ",          style = "muted" },
          { text = route.name,     style = "accent" } },
        { { text = route.days, style = "muted" } },
        { { text = "" } },
        { { text = route.summary, style = "body" } },
        { { text = "" } },
        { { text = "Itinerary:", style = "muted" } },
    }
    -- Highlight stops only during the actual stop loop. Pre / post
    -- overview phases don't single out a row — they're whole-route
    -- moments.
    local current_idx = 0
    if state.tour and state.tour.route == route
        and (state.tour.phase == "stop_fly" or state.tour.phase == "stop_dwell") then
        current_idx = state.tour.stop_idx
    end
    for i, s in ipairs(route.stops) do
        local marker, name_style
        if i == current_idx then
            marker, name_style = "▶ ", "accent"
        elseif state.tour and state.tour.route == route
            and (state.tour.phase == "post_fly" or state.tour.phase == "post_dwell"
                 or i < current_idx) then
            marker, name_style = "✓ ", "accent_alt"
        else
            marker, name_style = "  ", "accent_alt"
        end
        table.insert(lines, {
            { text = marker, style = "body" },
            { text = string.format("%d. %s", i, s.name), style = name_style },
        })
        table.insert(lines, {
            { text = "    " .. s.note, style = "body" },
        })
    end
    table.insert(lines, { { text = "" } })
    if state.tour and state.tour.route == route then
        table.insert(lines, { { text = phase_label_for(state.tour), style = "muted" } })
    else
        table.insert(lines, { { text = "Enter: play tour",          style = "muted" } })
        table.insert(lines, { { text = "q / Esc / Backspace: back", style = "muted" } })
    end
    return lines
end

local function build_lines()
    if state.detail then
        return render_detail(state.detail)
    end
    -- Empty list never happens (we always have routes), so this is
    -- only reached if items() returned [] for some other reason.
    return {
        { { text = "No routes available.", style = "muted" } },
    }
end

local function build_items()
    if state.detail then return {} end
    local items = {}
    for _, route in ipairs(routes) do
        table.insert(items, {
            { { text = route.country, style = "muted" },
              { text = " · ",          style = "muted" },
              { text = route.name,     style = "accent" },
              { text = "  ",           style = "body" },
              { text = route.days,     style = "muted" } },
            { { text = "  " .. route.summary, style = "body" } },
        })
    end
    return items
end

local function open_panel()
    if state.w then return end
    local n = #routes
    tick_handle = ttymap.api.frame.on_tick(on_tick)
    state.w = ttymap.api.card.open({
        name = "travel",
        -- footer_hints is read once at construction (static table —
        -- the bridge expects a sequence, not a function), so list /
        -- detail / tour all share one general hint row.
        footer_hints = {
            { key = "C-n/C-p", label = "select" },
            { key = "Enter",   label = "details / play" },
            { key = "q / Esc", label = "back / close" },
        },
        render       = build_lines,
        items        = build_items,
        selected     = function() return state.selected end,
        handle_key = function(key)
            -- Tour active: any cancellation key drops back to detail
            -- mode. Manual pan / zoom is handled by the animation
            -- lib (its tolerance check fires director.on_cancel
            -- which clears state.tour); we only catch keys that
            -- don't reach the map (Esc, etc.).
            if state.tour then
                if sidebar.is_close_key(key) or key.code == "Backspace" then
                    cancel_tour_silent()
                    ttymap.notify("Tour cancelled")
                    return nil
                end
                return { ignore = true }
            end

            -- Detail mode: Enter plays, Esc / Backspace returns to list.
            if state.detail then
                if key.code == "Enter" then
                    start_tour(state.detail)
                    return nil
                end
                if sidebar.is_close_key(key) or key.code == "Backspace" then
                    state.detail = nil
                    return nil
                end
                return { ignore = true }
            end

            -- List mode: navigate + open detail.
            if sidebar.up_pressed(key) then
                state.selected = sidebar.cycle(state.selected, n, -1)
                return nil
            end
            if sidebar.down_pressed(key) then
                state.selected = sidebar.cycle(state.selected, n, 1)
                return nil
            end
            if key.code == "Enter" then
                state.detail = routes[state.selected]
                return nil
            end
            if sidebar.is_close_key(key) then
                close_panel()
                return nil
            end
            return { ignore = true }
        end,
    })
end

local function toggle()
    -- Re-pressing the palette command always exits completely:
    -- cancel any tour, drop detail, close panel. One keypress to
    -- dismiss everything regardless of which sub-state we're in.
    if state.tour or state.detail or state.w then
        cancel_tour_silent()
        state.detail = nil
        close_panel()
        return
    end
    open_panel()
end

ttymap.register_palette_command({
    label  = "Travel",
    invoke = toggle,
})
