-- ruler — measure great-circle distance + bearing from an anchored
-- point to the live mouse cursor.
--
-- UX: pressing `m` (or invoking `Toggle ruler` from the palette)
-- captures the current cursor position (or the map centre, when no
-- cursor is available) as the anchor and switches the per-tick
-- callback into "draw" mode. Each frame thereafter renders a polyline
-- from the anchor to the cursor and shows the live distance + 8-way
-- bearing label in the bottom-left, exactly where the scale bar
-- doesn't reach. A second press clears the anchor and stops drawing.
--
-- Cursor unavailable (mouse off the rendered map) → the polyline is
-- skipped for that frame but the anchor stays — moving the mouse
-- back over the map resumes the readout.

local fmt = require "ttymap.fmt"
local geo = require "ttymap.geo"

local state = {
    active     = false,
    anchor_lon = nil,
    anchor_lat = nil,
}

local function toggle()
    if state.active then
        state.active = false
        state.anchor_lon = nil
        state.anchor_lat = nil
        ttymap.notify("Ruler: off")
        return
    end
    local lon, lat = ttymap.map:cursor()
    if not lon then
        lon, lat = ttymap.map:center()
    end
    state.anchor_lon = lon
    state.anchor_lat = lat
    state.active = true
    ttymap.notify(string.format(
        "Ruler: anchored at %.4f, %.4f", lat, lon
    ))
end

ttymap.api.frame.on_tick(function(map)
    if not state.active then return end
    local clon, clat = map:cursor()
    if not clon then return end

    map:polyline({
        { state.anchor_lon, state.anchor_lat },
        { clon, clat },
    }, "accent")

    local d = geo.haversine_m(state.anchor_lat, state.anchor_lon, clat, clon)
    local b = geo.bearing_label(state.anchor_lat, state.anchor_lon, clat, clon)
    local label
    if b then
        label = string.format(" ruler: %s %s ", fmt.distance(d), b)
    else
        label = string.format(" ruler: %s ", fmt.distance(d))
    end
    map:text_anchored("bottom-left", 1, label, "accent")
end)

ttymap.register_palette_command({
    label = "Toggle ruler",
    hint  = "m",
    invoke = toggle,
})
ttymap.register_keybind("m", toggle)
