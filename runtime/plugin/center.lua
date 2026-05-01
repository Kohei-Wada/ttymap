-- center marker — toggle a fixed crosshair at the map's current
-- centre. The plugin lives as an always-on overlay; visibility is
-- a plugin-internal flag the activation callbacks flip. Setup
-- state is shared between the callbacks and the overlay's
-- `paint_on_map`, so `enabled = not enabled` in `register_keybind`
-- is visible on the next paint tick.

local enabled = false

ttymap.register_overlay({
    name = "center",
    paint_on_map = function(map)
        if not enabled then return end
        local lon, lat = map:center()
        map:point(lon, lat, "+", "accent_alt")
    end,
})

local function toggle()
    enabled = not enabled
end

ttymap.register_palette_command({
    label = "Toggle center marker",
    invoke = toggle,
})

ttymap.register_keybind("c", toggle)
