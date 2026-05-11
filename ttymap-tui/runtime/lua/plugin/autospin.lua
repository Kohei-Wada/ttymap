-- autospin — globe rotation. Toggle via the palette and the camera
-- drifts eastward at a constant per-tick rate, looping at the
-- antimeridian. The demo that says "this is a globe" without saying
-- it.
--
-- Detection of manual input mirrors `ttymap.animation`'s approach:
-- after each jump we remember the lon we dispatched and compare it
-- against the live centre next tick. If they diverge beyond a small
-- tolerance, the user moved the map and we yield — pressing `h` /
-- `j` / `k` / `l` / mouse-drag stops the spin without an extra
-- keystroke. Zoom changes don't affect lon, so `+` / `-` keep
-- spinning.

local SPEED_DEG_PER_TICK = 0.3   -- ~9°/sec at ~30 fps → ~40 s per revolution
local TOL_LON            = 0.001 -- looser than the tolerance in animation.lua
                                 -- (which guards two-axis float drift); we
                                 -- only watch lon, so we can be strict.

local state = {
    active       = false,
    tick_handle  = nil,
    expected_lon = nil,   -- last lon we dispatched; manual-pan detector
}

local function stop(reason)
    state.active = false
    state.expected_lon = nil
    if state.tick_handle then
        state.tick_handle:remove()
        state.tick_handle = nil
    end
    ttymap.notify("Autospin: " .. (reason or "off"))
end

local function start()
    state.active = true
    state.expected_lon = nil
    state.tick_handle = ttymap.api.frame.on_tick(function(map)
        if not state.active then return end
        local lon, lat = map:center()
        -- Manual-pan detector — if the live lon doesn't match what
        -- we dispatched last tick, someone else moved the map.
        if state.expected_lon
            and math.abs(lon - state.expected_lon) > TOL_LON then
            stop("yielded to manual pan")
            return
        end
        local new_lon = lon + SPEED_DEG_PER_TICK
        if new_lon > 180 then new_lon = new_lon - 360 end
        ttymap.map:jump(new_lon, lat)
        state.expected_lon = new_lon
    end)
    ttymap.notify("Autospin: on")
end

local function toggle()
    if state.active then stop() else start() end
end

ttymap.register_palette_command({
    label = "Toggle autospin",
    invoke = toggle,
})
