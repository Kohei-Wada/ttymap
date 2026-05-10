-- center marker — toggle a fixed crosshair at the map's current
-- centre. Toggle pattern: when enabled, an `on_tick` subscriber is
-- registered against the bus and its `EventHandle` is held; toggle
-- off calls `:remove()` so the callback is genuinely gone from the
-- bus iteration (no per-frame `if not enabled then return end`
-- early-return cost when the marker is hidden).

local handle = nil

local function toggle()
    if handle then
        handle:remove()
        handle = nil
    else
        handle = ttymap.api.frame.on_tick(function(map)
            local lon, lat = map:center()
            map:point(lon, lat, "+", "accent_alt")
        end)
    end
end

ttymap.register_palette_command({
    label = "Toggle center marker",
    hint = "c",
    invoke = toggle,
})

ttymap.register_keybind("c", toggle)
