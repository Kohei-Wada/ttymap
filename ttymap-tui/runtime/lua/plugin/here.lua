-- here — palette action that jumps to the user's IP-geolocated
-- coordinates.
--
-- Location lookup is shared via ttymap.location, so subsequent
-- presses in the same session (or after a recent ttymap run within
-- the lib's TTL window) skip the network round-trip.
--
-- Endpoint comes from `ttymap.here.endpoint` (defaults to ipapi.co;
-- override in init.lua via `require("ttymap.here").endpoint = "..."`).

local anim = require "ttymap.animation"
local loc  = require "ttymap.location"

ttymap.register_palette_command({
    label = "Jump to here (current location)",
    invoke = function()
        loc.get(function(lat, lon)
            -- nil on error — location lib already surfaced the warn
            -- via ttymap.notify, so we just drop it here.
            if not lat then return end
            anim.fly_to(lon, lat)
            ttymap.notify(string.format(
                "Flew to %.4f, %.4f", lat, lon
            ))
        end)
    end,
})
