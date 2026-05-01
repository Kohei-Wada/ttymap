-- center.lua — Always-on central marker for better visibility.
--
-- Displays a fixed crosshair at the map's current center point.
-- Useful as a reference when navigating or for precise positioning.

ttymap.register_plugin({
    name = "center",
    label = "Toggle center marker",
    key = "c",

    paint_on_map = function(map)
        local lon, lat = map:center()
        map:point(lon, lat, "+", "accent_alt")
    end,

    handle_event = function(_)
        -- This is a marker-only plugin; it has no panel to interact
        -- with. We return ignore = true so that navigation keys
        -- fall through to the map even when this plugin is on top
        -- of the stack (focused).
        return { ignore = true }
    end,
})
