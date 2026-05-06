-- ttymap.animation — frame-based camera animation library.
--
-- Wraps `ttymap.map:fly_to` into a multi-frame interpolated transition
-- so plugins that previously called `:jump(lon, lat)` can hand the
-- view off to an animated pan instead. The visible behaviour is "the
-- map glides toward the target over ~30 frames" — implemented by
-- dispatching a fresh `:fly_to(lerp_value)` every frame from
-- `on_tick`. Each per-tick dispatch is the existing instant composite,
-- so there are no intermediate frames at half-applied state.
--
-- Usage:
--   local anim = require "ttymap.animation"
--   anim.fly_to(lon, lat)                            -- pan only, default frames
--   anim.fly_to(lon, lat, zoom)                      -- with zoom change
--   anim.fly_to(lon, lat, zoom, { frames = 60 })     -- override duration
--
-- Cancellation: a single animation is in flight at a time. A new
-- `fly_to` call replaces the previous one (smoothly — the new `from`
-- is whatever centre/zoom the map happens to be at). Manual user
-- input (h/j/k/l pan, +/- zoom, mouse drag) interrupts the animation:
-- detected by comparing the live map state against the value we
-- dispatched last frame. If they diverge beyond tolerance, someone
-- else moved the map, so we yield.

local M = {}

-- Tolerances for "did the user touch it" detection. Loose enough to
-- absorb the float round-trip through `geo::normalize` + Mercator
-- clamps, tight enough that a single keystroke pan (one cell) trips
-- it.
local TOL_LL   = 0.0005
local TOL_ZOOM = 0.001

-- Default ~500ms at 60fps. Frame-count rather than ms because
-- `on_tick` doesn't surface dt; if the loop runs slower the
-- animation just takes longer wall-clock.
local DEFAULT_FRAMES = 30

local active = nil  -- { from, to, frames, elapsed, expected }

local function ease_in_out_cubic(t)
    if t < 0.5 then
        return 4 * t * t * t
    end
    local p = -2 * t + 2
    return 1 - p * p * p / 2
end

local function lerp(a, b, t)
    return a + (b - a) * t
end

-- Shortest-arc lon interpolation — same fix ping_simulation.lua uses
-- so e.g. Tokyo→NY traces over the Pacific (146°) instead of via
-- Eurasia (213°).
local function lerp_lon(a, b, t)
    local d = b - a
    if d > 180 then
        d = d - 360
    elseif d < -180 then
        d = d + 360
    end
    local lon = a + d * t
    if lon > 180 then
        lon = lon - 360
    elseif lon < -180 then
        lon = lon + 360
    end
    return lon
end

function M.fly_to(target_lon, target_lat, target_zoom, opts)
    opts = opts or {}
    local lon, lat = ttymap.map:center()
    local zoom = ttymap.map:zoom()
    active = {
        from     = { lon, lat, zoom },
        to       = { target_lon, target_lat, target_zoom or zoom },
        frames   = opts.frames or DEFAULT_FRAMES,
        elapsed  = 0,
        expected = nil,
    }
end

ttymap.api.frame.on_tick(function()
    if not active then return end

    -- Cancel if the map state diverged from what we dispatched last
    -- frame. Skipped on the first tick (expected == nil) since we
    -- haven't dispatched anything yet.
    if active.expected then
        local lon, lat = ttymap.map:center()
        local zoom = ttymap.map:zoom()
        if math.abs(lon - active.expected[1]) > TOL_LL
            or math.abs(lat - active.expected[2]) > TOL_LL
            or math.abs(zoom - active.expected[3]) > TOL_ZOOM then
            active = nil
            return
        end
    end

    active.elapsed = active.elapsed + 1
    local t = math.min(active.elapsed / active.frames, 1)
    local et = ease_in_out_cubic(t)
    local lon = lerp_lon(active.from[1], active.to[1], et)
    local lat = lerp(active.from[2], active.to[2], et)
    local z   = lerp(active.from[3], active.to[3], et)

    ttymap.map:fly_to(lon, lat, z)
    active.expected = { lon, lat, z }

    if t >= 1 then
        active = nil
    end
end)

return M
