-- snake — classic Snake game played on the world map. Eat fruit
-- placed on famous cities; the snake grows; bumping into yourself
-- or running off the polar walls ends the game.
--
-- Activation: `:` palette → "Snake game". Opens a small sidebar
-- card that captures arrow / hjkl keys for movement. Close the
-- card (q / Esc) to abort. Off by default — this is a game, not
-- a chrome plugin.
--
-- Mechanics:
--
--   * **Grid**: world is discretised into 10° × 10° cells (18 × 36
--     = 648 cells). Snake body is a list of cell coordinates.
--   * **Tick rate**: head advances every 20 frames (~3 steps/sec
--     at 60Hz).
--   * **Borders**: longitude wraps at the antimeridian; latitude is
--     hard-walled at ±60° (above/below the snake collapses on
--     Mercator and the game becomes unplayable).
--   * **Food**: drawn at the cell containing one of ~40 globally
--     recognisable cities. Eating snaps the score up, grows the
--     body by 3, and respawns the food at another city.
--   * **Death**: head enters a body cell, OR head crosses the
--     polar wall.
--
-- Camera is auto-flown to a world view on game start; after that
-- the player owns the camera (the game keeps painting wherever
-- the snake is — pan / zoom away if you want, the game continues
-- but you may lose sight of the snake). Standard ttymap "you're
-- in charge of the view" convention.

local sidebar = require "ttymap.sidebar"
local anim    = require "ttymap.animation"

local CELL_DEG        = 10   -- 10° per grid cell (18 × 36 = 648 cells worldwide)
local TICKS_PER_STEP  = 20   -- ~3 steps/sec at 60fps
local START_LON       = 0
local START_LAT       = 30
local INITIAL_GROWTH  = 3    -- cells gained per fruit

-- ── Famous cities — fruit spawns here. Spread across continents
--    so the snake is forced to cover ground. Coordinates snap to
--    the nearest 10° cell on spawn (multiple nearby cities can
--    map to the same cell — fine, the food just shows whichever
--    name was picked).
local CITIES = {
    -- Asia
    { lon = 139.69, lat =  35.69, name = "Tokyo" },
    { lon = 116.40, lat =  39.90, name = "Beijing" },
    { lon = 121.47, lat =  31.23, name = "Shanghai" },
    { lon = 114.16, lat =  22.32, name = "Hong Kong" },
    { lon = 100.50, lat =  13.75, name = "Bangkok" },
    { lon = 103.82, lat =   1.35, name = "Singapore" },
    { lon =  72.83, lat =  19.08, name = "Mumbai" },
    { lon =  77.10, lat =  28.70, name = "Delhi" },
    { lon = 126.98, lat =  37.57, name = "Seoul" },
    { lon = 120.98, lat =  14.60, name = "Manila" },
    { lon = 106.85, lat =  -6.21, name = "Jakarta" },
    { lon =  55.27, lat =  25.20, name = "Dubai" },
    -- Europe
    { lon =  -0.13, lat =  51.51, name = "London" },
    { lon =   2.35, lat =  48.86, name = "Paris" },
    { lon =  13.41, lat =  52.52, name = "Berlin" },
    { lon =  12.50, lat =  41.90, name = "Rome" },
    { lon =  -3.70, lat =  40.42, name = "Madrid" },
    { lon =  37.62, lat =  55.75, name = "Moscow" },
    { lon =  28.98, lat =  41.01, name = "Istanbul" },
    { lon =  23.73, lat =  37.98, name = "Athens" },
    { lon =  18.07, lat =  59.33, name = "Stockholm" },
    { lon =  16.37, lat =  48.21, name = "Vienna" },
    { lon =   4.90, lat =  52.37, name = "Amsterdam" },
    { lon = -21.94, lat =  64.15, name = "Reykjavik" },
    -- Americas
    { lon = -74.01, lat =  40.71, name = "New York" },
    { lon =-118.24, lat =  34.05, name = "Los Angeles" },
    { lon = -87.65, lat =  41.88, name = "Chicago" },
    { lon =-122.42, lat =  37.77, name = "San Francisco" },
    { lon = -99.13, lat =  19.43, name = "Mexico City" },
    { lon = -58.38, lat = -34.61, name = "Buenos Aires" },
    { lon = -43.20, lat = -22.91, name = "Rio de Janeiro" },
    { lon = -77.04, lat = -12.05, name = "Lima" },
    { lon = -79.38, lat =  43.65, name = "Toronto" },
    { lon = -82.36, lat =  23.13, name = "Havana" },
    -- Africa
    { lon =  31.24, lat =  30.04, name = "Cairo" },
    { lon =   3.38, lat =   6.46, name = "Lagos" },
    { lon =  18.42, lat = -33.92, name = "Cape Town" },
    { lon =  36.82, lat =  -1.29, name = "Nairobi" },
    { lon =  -7.59, lat =  33.57, name = "Casablanca" },
    { lon =  38.74, lat =   9.03, name = "Addis Ababa" },
    -- Oceania
    { lon = 151.21, lat = -33.87, name = "Sydney" },
    { lon = 144.96, lat = -37.81, name = "Melbourne" },
    { lon = 174.76, lat = -36.85, name = "Auckland" },
}

local function snap(deg)
    return math.floor(deg / CELL_DEG + 0.5) * CELL_DEG
end

local function eq_cell(a, b)
    return a.lon == b.lon and a.lat == b.lat
end

-- ── Game state ───────────────────────────────────────────────────
local state = {
    enabled = false,
    body          = {},   -- { {lon, lat}, ... } head first
    direction     = "east",   -- "east" | "west" | "north" | "south"
    pending_dir   = nil,      -- queued direction change for next step
    food          = nil,      -- { lon, lat, name }
    score         = 0,
    tick          = 0,
    grow_pending  = 0,
    w             = nil,      -- sidebar card handle
    last_msg      = nil,      -- last "Eaten: <city>" / "Game over" line for sidebar
}

local function spawn_food()
    -- Try a handful of cities; if every cell collides with the
    -- body (improbable for a healthy snake length), give up and
    -- pick whatever — game balance over correctness.
    for _ = 1, 16 do
        local pick = CITIES[math.random(#CITIES)]
        local food = { lon = snap(pick.lon), lat = snap(pick.lat), name = pick.name }
        local hit = false
        for _, b in ipairs(state.body) do
            if eq_cell(b, food) then hit = true; break end
        end
        if not hit then
            state.food = food
            return
        end
    end
    -- Fallback: just take the last attempt even if it overlaps.
    -- A 40+ snake on a 648-cell board is the only way here.
end

local function next_pos(head, dir)
    local lon, lat = head.lon, head.lat
    if     dir == "east"  then lon = lon + CELL_DEG
    elseif dir == "west"  then lon = lon - CELL_DEG
    elseif dir == "north" then lat = lat + CELL_DEG
    elseif dir == "south" then lat = lat - CELL_DEG
    end
    -- Longitude wraps at ±180.
    if lon > 180 then lon = lon - 360
    elseif lon <= -180 then lon = lon + 360
    end
    -- Polar walls.
    if lat > 60 or lat < -60 then return nil end
    return { lon = lon, lat = lat }
end

local function close_card()
    if state.w then
        state.w:close()
        state.w = nil
    end
end

local function game_over(reason)
    state.enabled = false
    state.last_msg = string.format("Game over (%s) — Score: %d", reason, state.score)
    ttymap.notify(state.last_msg)
end

local function start()
    state.body         = { { lon = snap(START_LON), lat = snap(START_LAT) } }
    state.direction    = "east"
    state.pending_dir  = nil
    state.score        = 0
    state.tick         = 0
    state.grow_pending = 0
    state.last_msg     = nil
    spawn_food()
    state.enabled = true
    -- Glide to a world view so the whole board is visible. Player
    -- may pan / zoom freely after — the game keeps painting at the
    -- snake's actual coords.
    anim.fly_to(0, 20, 3)
end

-- Direction reversal blocker — can't go from east → west in one
-- step (would walk straight into the body).
local function is_reverse(a, b)
    return (a == "east"  and b == "west")
        or (a == "west"  and b == "east")
        or (a == "north" and b == "south")
        or (a == "south" and b == "north")
end

local function set_direction(d)
    if state.enabled and not is_reverse(state.direction, d) then
        state.pending_dir = d
    end
end

-- ── Per-frame: advance + paint ──────────────────────────────────
ttymap.api.frame.on_tick(function(map)
    if not state.enabled then
        -- Even after game over, keep the body painted while the
        -- card is still open so the player sees their final shape.
        if state.w and #state.body > 0 then
            for i, b in ipairs(state.body) do
                local glyph = (i == 1) and "●" or "○"
                local color = (i == 1) and "accent_alt" or "muted"
                map:point(b.lon, b.lat, glyph, color)
            end
        end
        return
    end

    state.tick = state.tick + 1
    if state.tick >= TICKS_PER_STEP then
        state.tick = 0
        if state.pending_dir then
            state.direction = state.pending_dir
            state.pending_dir = nil
        end
        local new_head = next_pos(state.body[1], state.direction)
        if not new_head then
            game_over("hit polar wall")
            return
        end
        for _, b in ipairs(state.body) do
            if eq_cell(b, new_head) then
                game_over("ran into yourself")
                return
            end
        end
        table.insert(state.body, 1, new_head)
        if state.grow_pending > 0 then
            state.grow_pending = state.grow_pending - 1
        else
            table.remove(state.body)
        end
        if state.food and eq_cell(new_head, state.food) then
            state.score = state.score + 1
            state.grow_pending = state.grow_pending + INITIAL_GROWTH
            state.last_msg = string.format("🍎 %s — Score %d", state.food.name, state.score)
            ttymap.notify(state.last_msg)
            spawn_food()
        end
    end

    -- Draw snake.
    for i, b in ipairs(state.body) do
        local glyph = (i == 1) and "●" or "○"
        local color = (i == 1) and "accent" or "accent_alt"
        map:point(b.lon, b.lat, glyph, color)
    end
    -- Draw food + city label.
    if state.food then
        map:point(state.food.lon, state.food.lat, "🍎", "muted")
        map:label(state.food.lon, state.food.lat, state.food.name, "muted")
    end
end)

-- ── Sidebar card ────────────────────────────────────────────────
local function build_lines()
    local lines = {
        { { text = "Snake on the globe", style = "accent" } },
        { { text = "" } },
        { { text = string.format("Score:  %d", state.score), style = "body" } },
        { { text = string.format("Length: %d", #state.body), style = "body" } },
    }
    if state.food then
        table.insert(lines, { { text = string.format("Next: %s", state.food.name), style = "muted" } })
    end
    if state.last_msg then
        table.insert(lines, { { text = "" } })
        table.insert(lines, { { text = state.last_msg, style = "muted" } })
    end
    table.insert(lines, { { text = "" } })
    table.insert(lines, { { text = "← ↑ ↓ → / hjkl: move", style = "muted" } })
    table.insert(lines, { { text = "q / Esc: quit",        style = "muted" } })
    return lines
end

local function open_card()
    if state.w then return end
    state.w = ttymap.api.card.open({
        name = "snake",
        footer_hints = {
            { key = "arrows / hjkl", label = "move" },
            { key = "q / Esc",       label = "quit" },
        },
        render     = build_lines,
        handle_key = function(key)
            if sidebar.is_close_key(key) then
                state.enabled = false
                close_card()
                return nil
            end
            -- Direction keys: arrows + hjkl. Block reverse via
            -- set_direction. Anything else falls through to the
            -- base layer (so map pan / palette / theme switch
            -- still work mid-game — slightly cheating but useful
            -- for adjusting the view).
            local code = key.code
            if code == "Up"
                or (code == "Char" and key.char == "k") then
                set_direction("north"); return nil
            end
            if code == "Down"
                or (code == "Char" and key.char == "j") then
                set_direction("south"); return nil
            end
            if code == "Left"
                or (code == "Char" and key.char == "h") then
                set_direction("west"); return nil
            end
            if code == "Right"
                or (code == "Char" and key.char == "l") then
                set_direction("east"); return nil
            end
            return { ignore = true }
        end,
    })
end

local function toggle()
    if state.w then
        state.enabled = false
        close_card()
        return
    end
    start()
    open_card()
end

ttymap.register_palette_command({
    label  = "Snake game",
    invoke = toggle,
})
