-- ping_simulation.lua — reference plugin demonstrating animated polyline overlay.
--
-- Draws several ping lines growing from origin toward destination
-- over ~60 frames each, then pauses ~30 frames and restarts. Each
-- ping has an independent phase offset so they don't all flash at
-- the same time. Pure illustration of "cyber-attack visualisation"
-- use cases — swap the endpoints / colours / cadence for real data.
--
-- Toggle via the palette: `:` → "Toggle ping simulation". Off by default so
-- the screen stays clean unless the user opts in.
--
-- Each coord is { lon, lat } — same convention as map:point /
-- map:label. The `color` field is either an xterm-256 index
-- (0..255) or a keyword string ("accent" / "accent_alt" / "muted"
-- / "road"). Numeric indices give the plugin author full control;
-- keywords stay theme-aware across DARK / BRIGHT.

local pings = {
  { src = { 139.76,  35.68 }, dst = { -74.01,  40.71 }, color = 196, offset =  0 }, -- Tokyo  → New York   (red)
  { src = {  -0.13,  51.51 }, dst = { -46.63, -23.55 }, color = 208, offset = 18 }, -- London → São Paulo  (orange)
  { src = { 116.40,  39.90 }, dst = { 151.21, -33.87 }, color = 220, offset = 36 }, -- Beijing→ Sydney     (yellow)
  { src = {  37.62,  55.75 }, dst = {  18.42, -33.92 }, color =  51, offset = 54 }, -- Moscow → Cape Town  (cyan)
  { src = {  72.88,  19.08 }, dst = {-118.24,  34.05 }, color = 207, offset = 72 }, -- Mumbai → Los Angeles (magenta)
}

-- Animation phases per ping cycle:
--   [0           , out_steps        ): outbound — new line grows from src toward dst
--   [out_steps   , out_steps+ret    ): return — separate new line grows from dst toward src
--   [out_steps+ret, cycle           ): pause — nothing drawn
local out_steps = 45
local ret_steps = 45
local pause     = 20
local cycle     = out_steps + ret_steps + pause

local enabled = false
local frame   = 0

local function toggle()
  enabled = not enabled
  if enabled then
    -- Reset phase so the next activation starts the animation
    -- cleanly from the beginning rather than wherever the silent
    -- counter happened to land.
    frame = 0
  end
end

ttymap.register_palette_command({
  label  = "Toggle ping simulation",
  invoke = toggle,
})

-- Interpolate lon along the shortest-arc path around the globe so
-- e.g. Tokyo→NY traces over the Pacific (146°) instead of via
-- Eurasia/Atlantic (213°). Without this, a linearly-interpolated
-- lon crosses the antimeridian on the *long* side, which projects
-- as a discontinuous jump in views centred on negative longitudes
-- (the per-point projection in ll_to_subpixel takes the shortest
-- path from view centre, so the interpolated tip teleports across
-- the canvas mid-animation).
local function interp_lon(src_lon, dst_lon, t)
  local dlon = dst_lon - src_lon
  if dlon > 180 then
    dlon = dlon - 360
  elseif dlon < -180 then
    dlon = dlon + 360
  end
  local lon = src_lon + dlon * t
  if lon > 180 then
    lon = lon - 360
  elseif lon < -180 then
    lon = lon + 360
  end
  return lon
end

ttymap.api.frame.on_tick(function(map)
  if not enabled then
    return
  end
  frame = frame + 1
  for _, ping in ipairs(pings) do
    local i = (frame + ping.offset) % cycle
    if i == 0 then
      goto continue
    end
    if i <= out_steps then
      -- Outbound: a new line grows from src; tip interpolates src → dst.
      local t   = i / out_steps
      local lon = interp_lon(ping.src[1], ping.dst[1], t)
      local lat = ping.src[2] + (ping.dst[2] - ping.src[2]) * t
      map:polyline({ ping.src, { lon, lat } }, ping.color)
    elseif i <= out_steps + ret_steps then
      -- Return: a new line grows from dst; tip interpolates dst → src.
      -- (The previous outbound line is gone — overlays are ephemeral
      -- per frame, only what we push this frame is drawn.)
      local t   = (i - out_steps) / ret_steps
      local lon = interp_lon(ping.dst[1], ping.src[1], t)
      local lat = ping.dst[2] + (ping.src[2] - ping.dst[2]) * t
      map:polyline({ ping.dst, { lon, lat } }, ping.color)
    end
    -- else: pause phase, draw nothing.
    ::continue::
  end
end)
