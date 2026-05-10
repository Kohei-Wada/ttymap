-- geohash — xkcd #426 daily-destination plugin.
--
-- Press `m` (or `:` palette → "Geohash today") to:
--   1. Read the current map center and snap to its 1°×1° graticule
--      (truncate-toward-zero, NOT floor — see algorithm.graticule_of).
--   2. Fetch the day's DJIA opening from Crox's plain-text service
--      (cached aggressively — DJIA never changes once published).
--      West of -30° longitude the 30W rule kicks in: use the
--      previous calendar day's DJIA so participants don't have to
--      wait until NYSE opens.
--   3. MD5 "YYYY-MM-DD-DDDDD.DD" and split the digest into two hex
--      fractions → today's destination inside the graticule.
--   4. Drop a marker, fly the camera over, open a sidebar card with
--      target / DJIA / distance-from-you readouts.
--
-- The marker stays drawn every frame as long as a result is held;
-- closing the card with `q`/`Esc` only hides the panel — the marker
-- remains so the user can pan freely without losing the destination.
-- Re-pressing `m` re-fetches (cached, so instant on a repeat) and
-- re-flies.
--
-- See https://geohashing.site/geohashing/Main_Page for the
-- algorithm + cultural context. The "actually go there" angle is
-- the whole point: this plugin marks the spot, the user is supposed
-- to physically visit it (or as close as terrain / property lines
-- allow).

local algo    = require "plugin.geohash.algorithm"
local config  = require "ttymap.geohash"
local sidebar = require "ttymap.sidebar"
local anim    = require "ttymap.animation"

------------------------------------------------------------------
-- Module-level state
------------------------------------------------------------------

-- The most recently computed result (or nil before first invocation).
-- Drives the persistent map marker + the sidebar card body.
local current = nil  -- { date, djia_date, djia, lat_int, lon_int,
                     --   target_lat, target_lon, is_30w }

-- A fetch in flight (or nil). The on_tick callback drains the job's
-- response and promotes the resolved data into `current`.
local pending = nil  -- { date, djia_date, lat_int, lon_int, is_30w, job }

local card_handle = nil
local tick_handle = nil

------------------------------------------------------------------
-- Date / URL helpers
------------------------------------------------------------------

-- Geohashing is a local-calendar ritual — participants use whatever
-- "today" their wall clock shows, not a UTC-anchored date.
local function today_str()
    return os.date("%Y-%m-%d")
end

local function shift_day(date_str, delta_days)
    local y, mo, d = date_str:match("^(%d+)-(%d+)-(%d+)$")
    local t = os.time({
        year = tonumber(y), month = tonumber(mo), day = tonumber(d),
        hour = 12,  -- noon, well clear of any DST seam
    })
    return os.date("%Y-%m-%d", t + delta_days * 86400)
end

local function djia_url_for(date_str)
    local y, mo, d = date_str:match("^(%d+)-(%d+)-(%d+)$")
    return string.format(config.djia_url, y, mo, d)
end

-- Crox returns a single plain-text decimal number with a trailing
-- newline, e.g. "42158.22\n". Tolerate stray whitespace / extra
-- decoration just in case.
local function parse_djia(body)
    return body:match("([%d]+%.?[%d]*)")
end

local function format_distance(km)
    if km < 1 then return string.format("%.0f m",  km * 1000) end
    if km < 100 then return string.format("%.1f km", km)        end
    return string.format("%.0f km", km)
end

------------------------------------------------------------------
-- Sidebar card
------------------------------------------------------------------

local function build_lines()
    if pending then
        return {
            { { text = "Geohash · " .. pending.date,                  style = "accent" } },
            { { text = "Fetching DJIA " .. pending.djia_date .. "…", style = "muted"  } },
        }
    end
    if not current then
        return {
            { { text = "(no geohash yet)",                           style = "muted" } },
            { { text = "Press m or :Geohash today to compute today.", style = "muted" } },
        }
    end
    local center_lon, center_lat = ttymap.map:center()
    local dist = algo.haversine_km(
        center_lat, center_lon, current.target_lat, current.target_lon)
    local bearing = algo.bearing_8(
        center_lat, center_lon, current.target_lat, current.target_lon)
    local dist_str = format_distance(dist)
    return {
        { { text = "Geohash · " .. current.date, style = "accent" } },
        { { text = string.format("graticule  %d, %d",
            current.lat_int, current.lon_int), style = "body" } },
        { { text = string.format("target     %.4f, %.4f",
            current.target_lat, current.target_lon), style = "body" } },
        { { text = string.format("from you   %s %s",
            dist_str, bearing), style = "body" } },
        { { text = string.format("DJIA       %s%s",
            current.djia,
            current.is_30w and "  (30W rule)" or ""), style = "muted" } },
    }
end

local function close_card()
    if card_handle then
        card_handle:close()
        card_handle = nil
    end
end

local function open_card()
    if card_handle then return end
    card_handle = ttymap.api.card.open({
        name = "geohash",
        footer_hints = {
            { key = "Enter",   label = "fly to" },
            { key = "q / Esc", label = "close" },
        },
        render = build_lines,
        handle_key = function(key)
            if key.code == "Enter" and current then
                anim.fly_to(current.target_lon, current.target_lat)
                return nil
            end
            if sidebar.is_close_key(key) then
                close_card()
                return nil
            end
            return { ignore = true }
        end,
    })
end

------------------------------------------------------------------
-- Per-frame work — drain the in-flight DJIA fetch and paint the
-- destination marker. Subscribed once on first invocation; the
-- per-tick cost is trivial when there is no fetch and `current`
-- is unchanged, so leaving the subscription alive across the VM
-- lifetime is cheaper than juggling subscribe/unsubscribe.
------------------------------------------------------------------

local function on_tick(map)
    if pending and pending.job then
        local body = pending.job:try_take()
        if body then
            local djia = parse_djia(body)
            if not djia then
                ttymap.notify("geohash: bad DJIA response: "
                              .. body:sub(1, 80), { level = "warn" })
                pending = nil
                return
            end
            local target_lat, target_lon = algo.compute(
                pending.djia_date, djia,
                pending.lat_int, pending.lon_int)
            current = {
                date       = pending.date,
                djia_date  = pending.djia_date,
                djia       = djia,
                lat_int    = pending.lat_int,
                lon_int    = pending.lon_int,
                target_lat = target_lat,
                target_lon = target_lon,
                is_30w     = pending.is_30w,
            }
            pending = nil
            if not card_handle then open_card() end
            anim.fly_to(target_lon, target_lat)
            ttymap.notify(string.format(
                "geohash %s: %.4f, %.4f",
                current.date, target_lat, target_lon))
        end
    end
    if current then
        map:point(current.target_lon, current.target_lat,
                  config.marker_glyph, config.marker_color)
        map:label(current.target_lon, current.target_lat,
                  current.date, config.marker_color)
    end
end

------------------------------------------------------------------
-- Activation
------------------------------------------------------------------

local function compute_today()
    if pending then
        ttymap.notify("geohash: fetch already in flight")
        return
    end
    local center_lon, center_lat = ttymap.map:center()
    local lat_int = algo.graticule_of(center_lat)
    local lon_int = algo.graticule_of(center_lon)
    local date = today_str()

    -- 30W rule: west of -30° longitude, midnight local time happens
    -- before NYSE opens, so the day's DJIA isn't published yet.
    -- Fall back to the previous calendar day's value. Crox's service
    -- handles weekend / holiday rollback within its own response, so
    -- one day back is enough here.
    local djia_date, is_30w
    if lon_int < -30 then
        djia_date, is_30w = shift_day(date, -1), true
    else
        djia_date, is_30w = date, false
    end

    pending = {
        date       = date,
        djia_date  = djia_date,
        lat_int    = lat_int,
        lon_int    = lon_int,
        is_30w     = is_30w,
        job        = ttymap.http:fetch_cached(
                        djia_url_for(djia_date),
                        config.djia_ttl_s),
    }
    if not card_handle then open_card() end
    if not tick_handle then
        tick_handle = ttymap.api.frame.on_tick(on_tick)
    end
end

ttymap.register_keybind("m", compute_today)

ttymap.register_palette_command({
    label  = "Geohash today",
    hint   = "m",
    invoke = compute_today,
})
