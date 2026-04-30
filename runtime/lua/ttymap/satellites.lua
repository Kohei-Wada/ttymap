-- ttymap.satellites — factory for single-satellite marker plugins.
--
-- Used by `iss.lua`, `hubble.lua`, and any user-supplied plugin that
-- wants to track one named satellite by NORAD ID. The factory wraps:
--   1. TLE fetch (CelesTrak gp.php, FORMAT=tle, in-memory cache for
--      the session — disk cache + 24h refresh is future work).
--   2. SGP4 propagation via `ttymap.sgp4` (microseconds per call, so
--      we re-propagate every poll for frame-fresh motion).
--   3. ISS-style 30x4 panel + map marker, with Enter recentring on
--      the cached position.
--
-- Group plugins (Starlink etc.) are *not* this — they want
-- `ttymap.sgp4:propagate_batch` over thousands of TLEs and a
-- different UX. Keep them in their own file.

local M = {}

local function tle_url(norad_id)
    return string.format(
        "https://celestrak.org/NORAD/elements/gp.php?CATNR=%d&FORMAT=tle",
        norad_id)
end

--- Build a satellite-marker plugin module.
---
--- @param opts table fields:
---   display    string : panel + map-label name (e.g. "ISS")
---   norad_id   integer: CelesTrak / NORAD catalog ID
---   color      string?: palette key for marker + label (default "accent_alt")
function M.make(opts)
    local display = opts.display
    local norad_id = opts.norad_id
    local color = opts.color or "accent_alt"
    local url = tle_url(norad_id)

    local state = {
        tle = nil,            -- handle from ttymap.sgp4:parse_tle, once we've fetched
        position = nil,       -- last propagated { lon, lat, alt_km, vel_kms }
        fetch_job = nil,      -- pending HTTP job, nil otherwise
        initial_jump_done = false,
    }

    return {
        name = display:lower():gsub("%s+", "_"),
        label = "Toggle " .. display,

        -- Same footprint as the original iss panel so multiple
        -- satellite toggles stack cleanly at the top-left.
        layout = { anchor = "top-left", width = 30, height = 4 },

        render = function()
            local pos_line, alt_line
            if state.position then
                pos_line = string.format("%.2f°N, %.2f°E",
                    state.position.lat, state.position.lon)
                alt_line = string.format("%d km @ %.2f km/s",
                    math.floor(state.position.alt_km + 0.5),
                    state.position.vel_kms)
            else
                pos_line = state.tle and "(propagating…)" or "(awaiting TLE)"
                alt_line = ""
            end
            return {
                display,
                pos_line,
                alt_line,
            }
        end,

        paint_on_map = function(map)
            if state.position then
                map:point(state.position.lon, state.position.lat, "◉", color)
                map:label(state.position.lon, state.position.lat, " " .. display, color)
            end
        end,

        handle_event = function(key)
            -- Enter recentres on the cached position; pre-propagation
            -- we silently swallow it so the keystroke doesn't leak
            -- to the base layer mid-load.
            if key.code == "Enter" then
                if state.position then
                    ttymap.map:jump(state.position.lon, state.position.lat)
                end
                return nil
            end
            -- Non-modal: defer pan / zoom / quit to the base layer.
            return { ignore = true }
        end,

        poll = function()
            -- One-shot TLE fetch on first poll. Once the handle is
            -- parsed we keep it for the rest of the session — SGP4
            -- alone is good for hours, and the plugin is short-lived
            -- anyway since toggling off + on rebuilds the module.
            if not state.tle and not state.fetch_job then
                state.fetch_job = ttymap.http:fetch(url)
            end
            if state.fetch_job then
                local body = state.fetch_job:try_take()
                if body then
                    state.tle = ttymap.sgp4:parse_tle(body)
                    state.fetch_job = nil
                end
            end

            -- Re-propagate every poll. SGP4 runs in microseconds, so
            -- there's no rate-limit reason to throttle; passing nil
            -- for the time arg lets Rust use sub-second wall-clock,
            -- which keeps motion smooth even though `os.time()` is
            -- 1 s resolution.
            if state.tle then
                local pos = ttymap.sgp4:propagate(state.tle)
                if pos then
                    state.position = pos
                    -- Auto-recentre the first time a position
                    -- arrives so the marker is immediately visible
                    -- after the user toggles the plugin on.
                    if not state.initial_jump_done then
                        state.initial_jump_done = true
                        ttymap.map:jump(pos.lon, pos.lat)
                    end
                end
            end
        end,
    }
end

return M
