-- terminator — paint the day/night boundary as a polyline overlay,
-- with a `☀` marker at the subsolar point (noon) and a `☾` at the
-- antipode (midnight). Updated every frame from the system clock,
-- so the line drifts westward at ~15°/hour as Earth rotates.
--
-- Activation: `:` palette → "Toggle terminator". Off by default so
-- the screen stays clean unless the user opts in.
--
-- The math:
--
--   * **Solar declination δ** — how far north/south the sub-solar
--     point sits today. Standard ±23.45° approximation (cosine of
--     day-of-year). Accurate to ~0.5° year-round; the 1° polyline
--     resolution is the bigger error.
--   * **Subsolar longitude λ_sub** — at UTC noon, the sun is over
--     Greenwich (lon 0). Each hour later it's 15° westward.
--   * **Terminator at longitude λ** — the latitude φ where the sun
--     sits exactly on the horizon, derived from the dot product of
--     the subsolar zenith vector and the surface normal:
--
--         tan(φ) = -cos(λ - λ_sub) / tan(δ)
--
--     The curve is sinusoidal-looking on Mercator, oscillating
--     between ±|90° - δ| in latitude. Polar regions on the lit
--     hemisphere stay above the curve (full day), the unlit pole
--     stays below (full night) — which is exactly what happens at
--     each solstice.

local enabled = false

local function toggle()
    enabled = not enabled
end

ttymap.register_palette_command({
    label  = "Toggle terminator",
    invoke = toggle,
})

local function solar_declination_rad(yday)
    return math.rad(23.45) * math.sin(math.rad(360 / 365 * (284 + yday)))
end

-- Subsolar longitude in degrees. UTC noon → 0°, each hour westward
-- by 15°. Wraps via `((x + 540) % 360) - 180` to land in [-180, 180).
local function subsolar_lon_deg(utc_hour)
    local lon = -15 * (utc_hour - 12)
    return ((lon + 540) % 360) - 180
end

-- Recompute cache (polyline + sun/moon points) at 1Hz only —
-- the terminator drifts at 15° / hour ≈ 0.0042°/second, well
-- below the 1° polyline resolution. Recomputing every frame
-- would burn 21k sin/cos calls per second and produce visually
-- identical output. We still re-emit the cached overlays from
-- every on_tick, so panning / zooming sees the line right away.
local cache = {
    last_ts = -1,
    coords  = nil,
    sun     = nil,
    moon    = nil,
}

local function recompute()
    local now = os.date("!*t")  -- UTC
    local utc_hour = now.hour + now.min / 60 + now.sec / 3600

    local delta = solar_declination_rad(now.yday)
    local lon_sub = subsolar_lon_deg(utc_hour)

    -- At equinoxes δ ≈ 0 and `tan(δ) → 0`. The formula degenerates
    -- (terminator collapses to two meridians at λ_sub ± 90°). We
    -- floor tan(δ) at a small epsilon — the resulting curve is
    -- visually close to the true two-meridian shape and avoids
    -- division-by-zero.
    local tan_delta = math.tan(delta)
    if math.abs(tan_delta) < 0.001 then
        tan_delta = (tan_delta >= 0) and 0.001 or -0.001
    end

    local coords = {}
    for lon = -180, 180, 1 do
        local cos_diff = math.cos(math.rad(lon - lon_sub))
        local lat = math.deg(math.atan(-cos_diff / tan_delta))
        table.insert(coords, { lon, lat })
    end

    -- Subsolar point — the spot on Earth currently at solar noon.
    -- Latitude = solar declination (degrees from the equator).
    local lat_sub = math.deg(delta)
    -- Antisolar point — currently at solar midnight. Mirrors the
    -- subsolar through Earth's centre, so longitude is offset by
    -- 180° (wrapped into [-180, 180)) and latitude is negated.
    local lon_anti = ((lon_sub + 180 + 540) % 360) - 180

    cache.coords = coords
    cache.sun    = { lon_sub, lat_sub }
    cache.moon   = { lon_anti, -lat_sub }
end

ttymap.api.frame.on_tick(function(map)
    if not enabled then return end

    local now_ts = os.time()
    if now_ts ~= cache.last_ts then
        cache.last_ts = now_ts
        recompute()
    end

    map:polyline(cache.coords, "muted")
    map:point(cache.sun[1],  cache.sun[2],  "☀", "accent_alt")
    map:point(cache.moon[1], cache.moon[2], "☾", "muted")
end)
