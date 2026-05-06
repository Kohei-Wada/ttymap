-- geo_quiz — "find the city before time runs out" geography game.
--
-- A target city pops up; you have ~30 seconds to pan / zoom the
-- map so the **centre** lands as close to the city as possible.
-- Enter locks the guess (or the timer auto-locks at zero); the
-- camera flies out to a view that fits both the guess and the
-- target so the great-circle error is visible at a glance, with
-- ◎ markers and a connecting line. Enter starts the next round;
-- q / Esc quits.
--
-- Two difficulty modes wired as separate palette commands:
--
--   * **easy** — country shown alongside the city name.
--   * **hard** — city name only.
--
-- Score is a golf-style **cumulative km error** — lower is
-- better. The card shows the round's error + the running total
-- across the session, plus the average error per round.
--
-- ttymap-native by design: the map *is* the puzzle, panning is
-- the gameplay verb, and the reveal is a smooth animation rather
-- than a popup.

local sidebar = require "ttymap.sidebar"
local anim    = require "ttymap.animation"

-- 30s per round at 60fps. The timer ticks down once per
-- on_tick — a slower terminal just gets a longer wall-clock
-- round (acceptable, scoring doesn't depend on time anymore).
local TIME_LIMIT_FRAMES = 1800

-- Curated worldwide-recognisable cities. Spread across continents
-- so a random pick doesn't always land in Europe.
local CITIES = {
    -- Asia
    { lon = 139.69, lat =  35.69, name = "Tokyo",        country = "Japan" },
    { lon = 116.40, lat =  39.90, name = "Beijing",      country = "China" },
    { lon = 121.47, lat =  31.23, name = "Shanghai",     country = "China" },
    { lon = 114.16, lat =  22.32, name = "Hong Kong",    country = "China" },
    { lon = 100.50, lat =  13.75, name = "Bangkok",      country = "Thailand" },
    { lon = 103.82, lat =   1.35, name = "Singapore",    country = "Singapore" },
    { lon =  72.83, lat =  19.08, name = "Mumbai",       country = "India" },
    { lon =  77.10, lat =  28.70, name = "Delhi",        country = "India" },
    { lon = 126.98, lat =  37.57, name = "Seoul",        country = "South Korea" },
    { lon = 120.98, lat =  14.60, name = "Manila",       country = "Philippines" },
    { lon = 106.85, lat =  -6.21, name = "Jakarta",      country = "Indonesia" },
    { lon =  55.27, lat =  25.20, name = "Dubai",        country = "UAE" },
    { lon =  46.68, lat =  24.71, name = "Riyadh",       country = "Saudi Arabia" },
    { lon =  35.21, lat =  31.77, name = "Jerusalem",    country = "Israel" },
    -- Europe
    { lon =  -0.13, lat =  51.51, name = "London",       country = "United Kingdom" },
    { lon =   2.35, lat =  48.86, name = "Paris",        country = "France" },
    { lon =  13.41, lat =  52.52, name = "Berlin",       country = "Germany" },
    { lon =  12.50, lat =  41.90, name = "Rome",         country = "Italy" },
    { lon =  -3.70, lat =  40.42, name = "Madrid",       country = "Spain" },
    { lon =  37.62, lat =  55.75, name = "Moscow",       country = "Russia" },
    { lon =  28.98, lat =  41.01, name = "Istanbul",     country = "Turkey" },
    { lon =  23.73, lat =  37.98, name = "Athens",       country = "Greece" },
    { lon =  18.07, lat =  59.33, name = "Stockholm",    country = "Sweden" },
    { lon =  16.37, lat =  48.21, name = "Vienna",       country = "Austria" },
    { lon =   4.90, lat =  52.37, name = "Amsterdam",    country = "Netherlands" },
    { lon = -21.94, lat =  64.15, name = "Reykjavik",    country = "Iceland" },
    { lon =  -9.14, lat =  38.72, name = "Lisbon",       country = "Portugal" },
    -- Americas
    { lon = -74.01, lat =  40.71, name = "New York",     country = "USA" },
    { lon =-118.24, lat =  34.05, name = "Los Angeles",  country = "USA" },
    { lon = -87.65, lat =  41.88, name = "Chicago",      country = "USA" },
    { lon =-122.42, lat =  37.77, name = "San Francisco",country = "USA" },
    { lon = -99.13, lat =  19.43, name = "Mexico City",  country = "Mexico" },
    { lon = -58.38, lat = -34.61, name = "Buenos Aires", country = "Argentina" },
    { lon = -43.20, lat = -22.91, name = "Rio de Janeiro",country = "Brazil" },
    { lon = -46.63, lat = -23.55, name = "São Paulo",    country = "Brazil" },
    { lon = -77.04, lat = -12.05, name = "Lima",         country = "Peru" },
    { lon = -79.38, lat =  43.65, name = "Toronto",      country = "Canada" },
    { lon = -82.36, lat =  23.13, name = "Havana",       country = "Cuba" },
    { lon = -70.65, lat = -33.45, name = "Santiago",     country = "Chile" },
    -- Africa
    { lon =  31.24, lat =  30.04, name = "Cairo",        country = "Egypt" },
    { lon =   3.38, lat =   6.46, name = "Lagos",        country = "Nigeria" },
    { lon =  18.42, lat = -33.92, name = "Cape Town",    country = "South Africa" },
    { lon =  36.82, lat =  -1.29, name = "Nairobi",      country = "Kenya" },
    { lon =  -7.59, lat =  33.57, name = "Casablanca",   country = "Morocco" },
    { lon =  38.74, lat =   9.03, name = "Addis Ababa",  country = "Ethiopia" },
    -- Oceania
    { lon = 151.21, lat = -33.87, name = "Sydney",       country = "Australia" },
    { lon = 144.96, lat = -37.81, name = "Melbourne",    country = "Australia" },
    { lon = 174.76, lat = -36.85, name = "Auckland",     country = "New Zealand" },
}

-- Haversine — proper great-circle distance.
local function distance_km(a, b)
    local R = 6371
    local lat1 = math.rad(a.lat)
    local lat2 = math.rad(b.lat)
    local dlat = lat2 - lat1
    local dlon = math.rad(b.lon - a.lon)
    local h = math.sin(dlat / 2)^2 + math.cos(lat1) * math.cos(lat2) * math.sin(dlon / 2)^2
    return 2 * R * math.asin(math.min(1, math.sqrt(h)))
end

-- Pick a (centre, zoom) that frames both points with a bit of
-- margin so the reveal shows the whole error at a glance.
-- Mirrors travel's overview_view heuristic.
local function reveal_view(p1, p2)
    -- Antimeridian-aware mid-longitude.
    local dlon = p2.lon - p1.lon
    if dlon > 180 then
        dlon = dlon - 360
    elseif dlon < -180 then
        dlon = dlon + 360
    end
    local mid_lon = p1.lon + dlon / 2
    if mid_lon > 180 then mid_lon = mid_lon - 360
    elseif mid_lon < -180 then mid_lon = mid_lon + 360 end
    local mid_lat = (p1.lat + p2.lat) / 2

    local span_lon = math.abs(dlon)
    local span_lat = math.abs(p2.lat - p1.lat)
    local span = math.max(span_lon, span_lat, 0.5)
    -- 8.5 - log2(span) clamped to [1, 10]:
    --   span ≈ 1°  → zoom 8 (close, eg same metro area)
    --   span ≈ 10° → zoom 5 (regional)
    --   span ≈ 90° → zoom 2 (continental)
    local zoom = math.floor(8.5 - math.log(span) / math.log(2))
    if zoom < 1 then zoom = 1 elseif zoom > 10 then zoom = 10 end
    return mid_lon, mid_lat, zoom
end

local state = {
    difficulty   = "easy",      -- "easy" | "hard"
    target       = nil,         -- { lon, lat, name, country }
    timer_frames = 0,
    submitted    = false,
    result       = nil,         -- { distance_km, frames_used, guess }
    total_km     = 0,
    rounds       = 0,
    w            = nil,
}

local function pick_target()
    return CITIES[math.random(#CITIES)]
end

local function start_round()
    state.target       = pick_target()
    state.timer_frames = TIME_LIMIT_FRAMES
    state.submitted    = false
    state.result       = nil
    -- Snap to a world view — every round starts from the same
    -- vantage point so the puzzle is about navigation, not where
    -- the camera happened to be.
    ttymap.map:fly_to(0, 20, 1)
end

local function close_card()
    if state.w then
        state.w:close()
        state.w = nil
    end
end

local function submit()
    if state.submitted or not state.target then return end
    state.submitted = true
    local guess_lon, guess_lat = ttymap.map:center()
    local guess = { lon = guess_lon, lat = guess_lat }
    local d = distance_km(guess, state.target)
    state.result = {
        distance_km = d,
        frames_used = TIME_LIMIT_FRAMES - state.timer_frames,
        guess       = guess,
    }
    state.total_km = state.total_km + d
    state.rounds   = state.rounds + 1
    -- Reveal: fly to a view that frames both the target and the
    -- guess so the error is visible at a glance.
    local mid_lon, mid_lat, zoom = reveal_view(guess, state.target)
    anim.fly_to(mid_lon, mid_lat, zoom)
end

ttymap.api.frame.on_tick(function(map)
    if not state.target then return end

    -- Reveal painting. Pre-submit nothing is drawn — that would
    -- defeat the puzzle.
    if state.submitted then
        local t = state.target
        map:point(t.lon, t.lat, "◎", "accent")
        map:label(t.lon, t.lat, t.name, "accent")
        if state.result and state.result.guess then
            local g = state.result.guess
            map:point(g.lon, g.lat, "◎", "accent_alt")
            map:label(g.lon, g.lat, "your guess", "accent_alt")
            -- Polyline connecting guess → target so the player
            -- sees the magnitude of the error even when the two
            -- markers don't fit the same screenful.
            map:polyline({
                { g.lon, g.lat },
                { t.lon, t.lat },
            }, "muted")
        end
        return
    end

    -- Live round — tick the timer; auto-submit at zero.
    state.timer_frames = state.timer_frames - 1
    if state.timer_frames <= 0 then
        state.timer_frames = 0
        submit()
    end
end)

local function build_lines()
    local t = state.target
    if not t then
        return { { { text = "Press Enter to start", style = "muted" } } }
    end

    if state.submitted and state.result then
        local r = state.result
        local avg = state.total_km / state.rounds
        local target_label = string.format("%s, %s", t.name, t.country)
        return {
            { { text = "Result", style = "accent" } },
            { { text = "" } },
            { { text = string.format("Target:    %s", target_label), style = "body" } },
            { { text = string.format("Distance:  %d km",
                math.floor(r.distance_km + 0.5)), style = "accent_alt" } },
            { { text = string.format("Time used: %.1f s",
                r.frames_used / 60), style = "muted" } },
            { { text = "" } },
            { { text = string.format("Total: %d km / %d round%s",
                math.floor(state.total_km + 0.5), state.rounds,
                state.rounds == 1 and "" or "s"), style = "muted" } },
            { { text = string.format("Avg:   %d km / round",
                math.floor(avg + 0.5)), style = "muted" } },
            { { text = "" } },
            { { text = "Enter: next round",  style = "muted" } },
            { { text = "q / Esc: quit",      style = "muted" } },
        }
    end

    -- Live round — header, target, timer, instructions.
    local seconds = math.ceil(state.timer_frames / 60)
    local timer_style = (seconds <= 5) and "accent" or "body"
    local mode_label = (state.difficulty == "hard") and "HARD" or "easy"
    local lines = {
        { { text = "Find this city",   style = "muted" },
          { text = " · ",               style = "muted" },
          { text = mode_label,          style = "accent_alt" } },
        { { text = "" } },
        { { text = t.name, style = "accent" } },
    }
    -- Easy mode shows the country as a soft hint; hard hides it.
    if state.difficulty == "easy" then
        table.insert(lines, { { text = t.country, style = "muted" } })
    end
    table.insert(lines, { { text = "" } })
    table.insert(lines, { { text = string.format("Time: %d s", seconds), style = timer_style } })
    table.insert(lines, { { text = "" } })
    table.insert(lines, { { text = "Pan / zoom to position the centre.", style = "muted" } })
    table.insert(lines, { { text = "Enter: lock your guess",             style = "muted" } })
    table.insert(lines, { { text = "q / Esc: quit",                      style = "muted" } })
    return lines
end

local function open_card()
    if state.w then return end
    state.w = ttymap.api.card.open({
        name = "geo_quiz",
        footer_hints = {
            { key = "hjkl / arrows / mouse", label = "pan" },
            { key = "Enter",                 label = "lock / next" },
            { key = "q / Esc",               label = "quit" },
        },
        render     = build_lines,
        handle_key = function(key)
            if sidebar.is_close_key(key) then
                close_card()
                state.target = nil
                return nil
            end
            if key.code == "Enter" then
                if state.submitted then
                    start_round()
                else
                    submit()
                end
                return nil
            end
            -- All other keys (hjkl, arrows, +/-, mouse-derived)
            -- pass through to the base layer — pan / zoom IS the
            -- gameplay.
            return { ignore = true }
        end,
    })
end

local function start_session(difficulty)
    if state.w then
        close_card()
        state.target = nil
        return
    end
    state.difficulty = difficulty
    state.total_km   = 0
    state.rounds     = 0
    start_round()
    open_card()
end

ttymap.register_palette_command({
    label  = "Geo quiz · easy",
    invoke = function() start_session("easy") end,
})

ttymap.register_palette_command({
    label  = "Geo quiz · hard",
    invoke = function() start_session("hard") end,
})
