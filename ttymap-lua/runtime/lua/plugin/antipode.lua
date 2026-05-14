-- antipode — palette action that flies the camera to the
-- diametrically opposite point on the sphere.
--
-- Useful for the "if I dug straight down, where would I come out?"
-- moment. Most of the time it's the middle of an ocean — Tokyo's
-- antipode is empty Atlantic south of Argentina, Sydney's is the
-- North Atlantic, etc. ttymap's hemispheric-coverage tile fetch
-- + Mercator-aware fly_to handle the cross-globe pan cleanly.

local anim = require "ttymap.animation"
local geo  = require "ttymap.geo"

ttymap.register_palette_command({
    label = "Fly to antipode",
    invoke = function()
        local lon, lat = ttymap.map:center()
        local alat, alon = geo.antipode(lat, lon)
        anim.fly_to(alon, alat)
        ttymap.notify(string.format(
            "Antipode: %.4f, %.4f", alat, alon
        ))
    end,
})
