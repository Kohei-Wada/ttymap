-- center.lua — Always-on central marker for better visibility.
--
-- Displays a fixed crosshair at the map's current center point.
-- Useful as a reference when navigating or for precise positioning.

-- center marker — explicit opt-in for palette + keybind.
-- Activation surfaces are separate from the plugin definition; the
-- plugin author controls when (and whether) to push via callbacks.

ttymap.register_plugin({
    name = "center",

    paint_on_map = function(map)
        local lon, lat = map:center()
        map:point(lon, lat, "+", "accent_alt")
    end,

    handle_event = function(_)
        -- Marker-only plugin; let navigation keys fall through.
        return { ignore = true }
    end,
})

-- Add a row to the `:` palette. The callback returns true to push
-- a fresh component, false to skip — that's the hook for plugin-
-- side state management. Trivially "always push" here; a true
-- toggle would track an `opened` flag and gate accordingly.
ttymap.register_palette_command({
    label = "Toggle center marker",
    invoke = function() return true end,
})

-- Activation key. Same callback shape as register_palette_command.
ttymap.register_keybind("c", function() return true end)
