-- ping.lua — reference plugin demonstrating animated polyline overlay.
--
-- Draws a single ping line growing from Tokyo toward New York over
-- ~60 frames, then pauses ~30 frames and restarts. Pure illustration:
-- swap the endpoints / cadence for real "cyber-attack visualisation"
-- use cases (consume an event queue you maintain in plugin-local
-- upvalues, push one polyline per active ping per frame).
--
-- Each coord is { lon, lat } — same convention as map:point /
-- map:label. The polyline appears in the *next* frame after the push
-- (ephemeral re-submit + 1-frame transport lag), which is invisible
-- at the timescales of typical ping animations.

local src   = { 139.76, 35.68 }   -- Tokyo (lon, lat)
local dst   = { -74.01, 40.71 }   -- New York
local steps = 60
local pause = 30
local i     = 0

ttymap.api.frame.on_tick(function(map)
  i = (i + 1) % (steps + pause)
  if i == 0 or i > steps then
    return
  end
  local t   = i / steps
  local lon = src[1] + (dst[1] - src[1]) * t
  local lat = src[2] + (dst[2] - src[2]) * t
  map:polyline({ src, { lon, lat } }, "accent")
end)
