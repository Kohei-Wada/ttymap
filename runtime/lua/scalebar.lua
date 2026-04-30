-- scalebar (Lua port) — bottom-right always-on chrome showing a
-- distance scale tied to the rendered frame's centre / zoom.
--
-- Mirrors `geo::scale_bar` from the Rust impl: pick a nice round
-- distance whose pixel-width is closest to ~1/5 of the screen, then
-- render a Unicode bar of that many cells with a metric label.

local fmt = require "ttymap.fmt"

local EARTH_RADIUS_M = 6378137.0  -- WGS-84 equatorial radius
local NICE_DISTANCES = {
    50, 100, 200, 500,
    1000, 2000, 5000,
    10000, 20000, 50000,
    100000, 200000, 500000,
    1000000, 2000000, 5000000,
}

local function meters_per_cell(lat_deg, zoom)
    -- 256 px per tile at integer zoom; each terminal cell == 2 px
    -- horizontally because the Braille canvas packs 2 sub-pixels
    -- per cell.
    local lat_rad = lat_deg * math.pi / 180
    local meters_per_pixel = (EARTH_RADIUS_M * 2 * math.pi * math.cos(lat_rad))
        / (256 * (2 ^ zoom))
    return meters_per_pixel * 2
end

local function pick_distance(target_meters)
    local best = NICE_DISTANCES[1]
    local best_err = math.abs(best - target_meters)
    for i = 2, #NICE_DISTANCES do
        local d = NICE_DISTANCES[i]
        local err = math.abs(d - target_meters)
        if err < best_err then
            best, best_err = d, err
        end
    end
    return best
end

local function clamp(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

return {
    name = "scalebar",
    activation = "overlay",

    paint_on_map = function(map)
        local _, lat = map:center()
        local zoom = map:zoom()
        local width = map:area_width()
        if width == 0 then return end

        local mpc = meters_per_cell(lat, zoom)
        local target_cells = math.max(width / 5, 4)
        local target_meters = target_cells * mpc
        local distance = pick_distance(target_meters)
        local cells = math.floor(distance / mpc + 0.5)
        cells = clamp(cells, 2, math.floor(width / 3))
        if cells < 2 then return end

        local bar = "├" .. string.rep("─", cells - 2) .. "┤ " .. fmt.distance(distance) .. " "
        map:text_anchored("bottom-right", 0, bar, "accent")
    end,
}
