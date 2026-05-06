-- geo_quiz — "find the city before time runs out" geography game.
--
-- A target city pops up; you have ~30 seconds to pan / zoom the
-- map so that its **centre** lands as close to the city as
-- possible. Enter locks your guess (or the timer auto-locks at
-- zero); the camera then flies to the real location and a sidebar
-- card shows the distance error + a 0-1000 score. Enter starts
-- the next round.
--
-- Activation: `:` palette → "Geo quiz". A focused sidebar card
-- captures Enter / q / Esc but lets every other key — h/j/k/l /
-- mouse / +-/* zoom — fall through to the base layer, so the
-- entire stock map nav surface IS the gameplay.
--
-- ttymap-native by design: the map *is* the puzzle, the score is
-- a real great-circle distance, and the reveal is a smooth
-- camera animation rather than a popup.

local sidebar = require "ttymap.sidebar"
local anim    = require "ttymap.animation"

-- Rounds last ~30s at 60fps. The timer ticks down once per
-- on_tick; a slower terminal just gets a longer wall-clock round
-- (acceptable — the score formula doesn't care, it counts
-- frames-used not wall seconds).
local TIME_LIMIT_FRAMES = 1800
local REVEAL_ZOOM       = 8

-- Curated worldwide-recognisable cities. Same shape as the
-- travel plugin's stop list. Spread across continents so a
-- random pick doesn't always land in Europe.
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

-- Haversine — proper great-circle distance. The lat-only and
-- equirectangular shortcuts blow up over hemispheres, and the
-- score reveals demand a real number anyway.
local function distance_km(a, b)
    local R = 6371
    local lat1 = math.rad(a.lat)
    local lat2 = math.rad(b.lat)
    local dlat = lat2 - lat1
    local dlon = math.rad(b.lon - a.lon)
    local h = math.sin(dlat / 2)^2 + math.cos(lat1) * math.cos(lat2) * math.sin(dlon / 2)^2
    return 2 * R * math.asin(math.min(1, math.sqrt(h)))
end

-- Score: distance bonus drops linearly to 0 over 5000 km of
-- error (≈ Tokyo ↔ Mumbai). Time bonus drops linearly to 0 over
-- the round's full duration. Max 2000.
local function score_for(distance, frames_used)
    local d = math.max(0, math.floor(1000 * (1 - distance / 5000)))
    local t = math.max(0, math.floor(1000 * (1 - frames_used / TIME_LIMIT_FRAMES)))
    return d + t
end

local state = {
    target       = nil,   -- { lon, lat, name, country }
    timer_frames = 0,
    submitted    = false,
    result       = nil,   -- { distance_km, frames_used, score, guess }
    total_score  = 0,
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
    -- Snap to a world view so every round starts from the same
    -- vantage point — fair across rounds, and makes the puzzle
    -- about navigation, not "where did the camera happen to be".
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
    local frames_used = TIME_LIMIT_FRAMES - state.timer_frames
    local s = score_for(d, frames_used)
    state.result = {
        distance_km = d,
        frames_used = frames_used,
        score       = s,
        guess       = guess,
    }
    state.total_score = state.total_score + s
    state.rounds      = state.rounds + 1
    -- Reveal — animate to the real city so the result feels like
    -- an answer, not a lookup.
    anim.fly_to(state.target.lon, state.target.lat, REVEAL_ZOOM)
end

ttymap.api.frame.on_tick(function(map)
    if not state.target then return end

    -- Always paint the target marker once submitted (so the
    -- player sees where they should have aimed). Pre-submit, no
    -- marker — that would defeat the puzzle.
    if state.submitted then
        map:point(state.target.lon, state.target.lat, "★", "accent")
        map:label(state.target.lon, state.target.lat, state.target.name, "accent")
        if state.result and state.result.guess then
            map:point(state.result.guess.lon, state.result.guess.lat, "✗", "accent_alt")
        end
        return
    end

    -- Tick the round timer. At zero, lock the guess at whatever
    -- the camera was last looking at.
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
        return {
            { { text = "Result", style = "accent" } },
            { { text = "" } },
            { { text = string.format("Target:    %s, %s", t.name, t.country), style = "body" } },
            { { text = string.format("Distance:  %d km", math.floor(r.distance_km + 0.5)), style = "body" } },
            { { text = string.format("Time used: %.1f s",  r.frames_used / 60), style = "body" } },
            { { text = string.format("Score:     %d / 2000", r.score), style = "accent_alt" } },
            { { text = "" } },
            { { text = string.format("Total: %d over %d round%s",
                state.total_score, state.rounds,
                state.rounds == 1 and "" or "s"), style = "muted" } },
            { { text = "" } },
            { { text = "Enter: next round", style = "muted" } },
            { { text = "q / Esc: quit",     style = "muted" } },
        }
    end

    -- Live round.
    local seconds = math.ceil(state.timer_frames / 60)
    local timer_style = (seconds <= 5) and "accent" or "body"
    return {
        { { text = "Find this city", style = "muted" } },
        { { text = "" } },
        { { text = t.name,    style = "accent" } },
        { { text = t.country, style = "muted" } },
        { { text = "" } },
        { { text = string.format("Time: %d s", seconds), style = timer_style } },
        { { text = "" } },
        { { text = "Pan / zoom to position the centre.", style = "muted" } },
        { { text = "Enter: lock your guess",             style = "muted" } },
        { { text = "q / Esc: quit",                      style = "muted" } },
    }
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

ttymap.register_palette_command({
    label  = "Geo quiz",
    invoke = function()
        if state.w then
            close_card()
            state.target = nil
            return
        end
        state.total_score = 0
        state.rounds      = 0
        start_round()
        open_card()
    end,
})
