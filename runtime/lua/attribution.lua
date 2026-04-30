-- attribution — bottom-left always-on chrome showing the tile
-- provider's attribution text.
--
-- The active TileClient's attribution is read from the host at paint
-- time, so this plugin is a pure Lua file with no Rust glue. If the
-- provider returns nil/empty (custom backends without OSM data),
-- nothing is painted.

ttymap.register_plugin({
    name = "attribution",
    activation = "overlay",

    paint_on_map = function(map)
        local text = ttymap.tile:attribution()
        if text and #text > 0 then
            map:text_anchored("bottom-left", 0, text, "muted")
        end
    end,
})
