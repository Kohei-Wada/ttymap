-- equator — toggleable horizontal line at lat = 0.
--
-- Especially handy when `autospin` is running — a stationary line at
-- the rotation axis makes "the globe is spinning" read instantly
-- rather than "the labels are sliding for some reason". Off by
-- default so the screen stays clean unless the user opts in.
--
-- Drawn as a single polyline of 1° lon steps across the full -180..180
-- range, sampled densely enough that Mercator's straight-equator
-- mapping doesn't show stair-stepping at any zoom.

local SAMPLES = 361   -- one vertex per degree, inclusive of both ends

local handle = nil
local cached_coords = nil

local function build_coords()
    if cached_coords then return cached_coords end
    local coords = {}
    for i = 1, SAMPLES do
        coords[i] = { -180 + (i - 1), 0 }
    end
    cached_coords = coords
    return coords
end

local function start()
    if handle then return end
    handle = ttymap.api.frame.on_tick(function(map)
        map:polyline(build_coords(), "accent_alt")
    end)
    ttymap.notify("Equator: on")
end

local function stop()
    if not handle then return end
    handle:remove()
    handle = nil
    ttymap.notify("Equator: off")
end

ttymap.register_palette_command({
    label = "Toggle equator",
    invoke = function()
        if handle then stop() else start() end
    end,
})
