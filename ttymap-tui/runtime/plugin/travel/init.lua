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
--   3. **tour**: three sub-phases drive the camera. **pre** flies
--      to a bounding-box overview so the user sees the whole trip
--      laid out before plunging in (no jarring teleport-to-Tokyo);
--      **stop** loops through each `route.stops` entry, firing a
--      `ttymap.notify` per arrival; **post** returns to overview
--      for a final wrap-up beat. Manual pan / zoom (or palette
--      re-toggle) cancels at any phase. No polyline during tour
--      (Braille pixel-snapping ripples a static line as the camera
--      glides) — markers carry the journey on their own; future
--      stops show muted, current accent, past accent_alt.
--
-- Activation: `:` palette → "Travel". Re-invoking always exits
-- completely (cancels any tour, closes the panel).
--
-- Adding a country: see `travel/routes/init.lua`.

local sidebar   = require("ttymap.sidebar")
local anim      = require("ttymap.animation")
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
local PRE_DWELL_FRAMES  = 120   -- ~2s overview before the journey starts
local DWELL_FRAMES      = 120   -- ~2s parked at each stop
local POST_DWELL_FRAMES = 120   -- ~2s overview after final stop

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
    -- 8.5 - log2(span) clamped to [3, 11]:
    --   span ≥ 11°  → zoom 5    (continent / large country)
    --   span ≈  3°  → zoom 7    (Italian classic, Snow Monkey + Alps)
    --   span ≈  1°  → zoom 8-9  (Hokkaido, Sicily)
    --   span ≈ 0.5° → zoom 9-10 (Amalfi)
    local zoom = math.floor(8.5 - math.log(span) / math.log(2))
    if zoom < 3 then zoom = 3 elseif zoom > 11 then zoom = 11 end
    return center_lon, center_lat, zoom
end

local state = {
    selected = 1,    -- 1-based index into routes (list mode)
    w        = nil,  -- sidebar card handle while open
    detail   = nil,  -- nil = list mode; a route table = detail mode
    tour     = nil,  -- nil when not playing; else a tour table
    --                  { route, phase = "pre"|"stop"|"post",
    --                    stop_idx, dwell_left }
}

local function close_panel()
    if state.w then
        state.w:close()
        state.w = nil
    end
end

local advance_tour  -- forward decl

local function finish_tour(message)
    state.tour = nil
    if message then
        ttymap.notify(message)
    end
end

-- Generic on_cancel handler — every fly_to in the state machine bails
-- the whole tour the same way.
local function on_cancel_bail()
    finish_tour("Tour cancelled")
end

-- Schedule a fly_to whose `on_done` parks the camera for `dwell_frames`
-- before the next state-machine transition.
local function fly_then_dwell(lon, lat, zoom, dwell_frames)
    anim.fly_to(lon, lat, zoom, {
        on_done = function()
            if state.tour then
                state.tour.dwell_left = dwell_frames
            end
        end,
        on_cancel = on_cancel_bail,
    })
end

advance_tour = function()
    local t = state.tour
    if not t then return end

    if t.phase == "pre" then
        -- Pre-overview done. Enter stop loop at index 1.
        t.phase = "stop"
        t.stop_idx = 1
        local stop = t.route.stops[1]
        if not stop then
            finish_tour("Tour: empty route")
            return
        end
        ttymap.notify(stop.note)
        fly_then_dwell(stop.lon, stop.lat, stop.zoom, DWELL_FRAMES)
        return
    end

    if t.phase == "stop" then
        t.stop_idx = t.stop_idx + 1
        local stop = t.route.stops[t.stop_idx]
        if stop then
            ttymap.notify(stop.note)
            fly_then_dwell(stop.lon, stop.lat, stop.zoom, DWELL_FRAMES)
            return
        end
        -- All stops visited. Enter post-overview wrap-up.
        t.phase = "post"
        ttymap.notify(string.format("Tour complete: %s · %s",
            t.route.country, t.route.name))
        local lon, lat, z = overview_view(t.route)
        fly_then_dwell(lon, lat, z, POST_DWELL_FRAMES)
        return
    end

    -- Post-overview dwell expired — tour is done.
    finish_tour(nil)
end

local function start_tour(route)
    state.tour = {
        route      = route,
        phase      = "pre",
        stop_idx   = 0,
        dwell_left = 0,
    }
    ttymap.notify(string.format("Starting: %s · %s",
        route.country, route.name))
    local lon, lat, z = overview_view(route)
    fly_then_dwell(lon, lat, z, PRE_DWELL_FRAMES)
end

-- Per-frame map paint. Three cases, in priority order:
--
--   * **tour active**: paint markers for every stop, coloured by
--     phase + position (current accent, past accent_alt, future
--     muted). Future stops let the user see what's coming. No
--     polyline (Braille pixel-snapping ripples a static line as
--     the camera glides). Tour state advances via `advance_tour`
--     from the dwell countdown here and from the `on_done` callback
--     in `fly_then_dwell`.
--   * **detail mode**: no tour, but a route is selected — paint the
--     whole polyline + all markers as a preview. Camera is static
--     here (the user hasn't started flying), so the line is stable.
--   * **list mode**: nothing to paint.
ttymap.api.frame.on_tick(function(map)
    local t = state.tour
    if t then
        local route = t.route
        local current_idx = (t.phase == "stop") and t.stop_idx or 0

        -- Route polyline: visible only during the pre/post overview
        -- *dwells* (camera parked at the bbox view = stable line).
        -- The pre dwell is the "here is the whole journey" moment
        -- before the stop-by-stop flight; the post dwell mirrors it
        -- as a wrap-up. Hidden during all camera motion (pre fly,
        -- stop fly, post fly) because Braille pixel-snapping ripples
        -- a static line as the camera glides.
        if (t.phase == "pre" or t.phase == "post") and t.dwell_left > 0 then
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
            elseif t.phase == "post" or i < current_idx then
                color = "accent_alt"
            else
                color = "muted"
            end
            map:point(s.lon, s.lat, "●", color)
            map:label(s.lon, s.lat, s.name, color)
        end
        if t.dwell_left > 0 then
            t.dwell_left = t.dwell_left - 1
            if t.dwell_left == 0 then
                advance_tour()
            end
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
end)

-- ── Detail-mode rendering ─────────────────────────────────────────
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
    if state.tour and state.tour.route == route and state.tour.phase == "stop" then
        current_idx = state.tour.stop_idx
    end
    for i, s in ipairs(route.stops) do
        local marker, name_style
        if i == current_idx then
            marker, name_style = "▶ ", "accent"
        elseif state.tour and state.tour.route == route
            and (state.tour.phase == "post" or i < current_idx) then
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
        local phase_label
        if state.tour.phase == "pre" then
            phase_label = "Overview — orienting…"
        elseif state.tour.phase == "post" then
            phase_label = "Wrap-up — overview…"
        else
            phase_label = "Tour playing — pan / zoom to cancel."
        end
        table.insert(lines, { { text = phase_label, style = "muted" } })
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
            -- lib (its tolerance check fires on_cancel which clears
            -- state.tour); we only catch keys that don't reach the
            -- map (Esc, etc.).
            if state.tour then
                if sidebar.is_close_key(key) or key.code == "Backspace" then
                    finish_tour("Tour cancelled")
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
        if state.tour then
            finish_tour(nil)
        end
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
