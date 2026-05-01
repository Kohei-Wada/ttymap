-- ping.lua — reference plugin demonstrating animated polyline overlay.
--
-- Draws several ping lines growing from origin toward destination
-- over ~60 frames each, then pauses ~30 frames and restarts. Each
-- ping has an independent phase offset so they don't all flash at
-- the same time. Pure illustration of "cyber-attack visualisation"
-- use cases — swap the endpoints / colours / cadence for real data.
--
-- Each coord is { lon, lat } — same convention as map:point /
-- map:label. The polyline appears in the *next* frame after the
-- push (ephemeral re-submit + 1-frame transport lag), which is
-- invisible at the timescales of typical ping animations.
--
-- The `color` field is either an xterm-256 index (0..255) or a
-- keyword string ("accent" / "accent_alt" / "muted" / "road").
-- Numeric indices give the plugin author full control over the
-- look; keywords stay theme-aware across DARK / BRIGHT.

local pings = {
  { src = { 139.76,  35.68 }, dst = { -74.01,  40.71 }, color = 196, offset =  0 }, -- Tokyo  → New York   (red)
  { src = {  -0.13,  51.51 }, dst = { -46.63, -23.55 }, color = 208, offset = 18 }, -- London → São Paulo  (orange)
  { src = { 116.40,  39.90 }, dst = { 151.21, -33.87 }, color = 220, offset = 36 }, -- Beijing→ Sydney     (yellow)
  { src = {  37.62,  55.75 }, dst = {  18.42, -33.92 }, color =  51, offset = 54 }, -- Moscow → Cape Town  (cyan)
  { src = {  72.88,  19.08 }, dst = {-118.24,  34.05 }, color = 207, offset = 72 }, -- Mumbai → Los Angeles (magenta)
}

local steps = 60
local pause = 30
local cycle = steps + pause

local frame = 0

ttymap.api.frame.on_tick(function(map)
  frame = frame + 1
  for _, ping in ipairs(pings) do
    -- Each ping's phase = (frame + offset) mod cycle. The active
    -- window is i ∈ (0, steps]; outside that the ping is paused.
    local i = (frame + ping.offset) % cycle
    if i == 0 or i > steps then
      goto continue
    end
    local t   = i / steps
    local lon = ping.src[1] + (ping.dst[1] - ping.src[1]) * t
    local lat = ping.src[2] + (ping.dst[2] - ping.src[2]) * t
    map:polyline({ ping.src, { lon, lat } }, ping.color)
    ::continue::
  end
end)
