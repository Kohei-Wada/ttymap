-- ttymap.satellites — multi-sat tracker as a single Component.
--
-- Each Component instance aggregates N satellites (configured by the
-- consumer), shares one panel for status display, and toggles per-sat
-- visibility via in-panel keystrokes. TLE fetch (CelesTrak) and SGP4
-- propagation run per visible sat; markers / labels paint only what's
-- visible. One palette entry, one window, regardless of how many
-- satellites the consumer configures.

local M = {}

local function tle_url(norad_id)
    return string.format(
        "https://celestrak.org/NORAD/elements/gp.php?CATNR=%d&FORMAT=tle",
        norad_id)
end

local function format_position(pos)
    if not pos then return "(awaiting…)" end
    return string.format("%.1f°N, %.1f°E  %dkm",
        pos.lat, pos.lon,
        math.floor(pos.alt_km + 0.5))
end

--- Build a multi-satellite tracker plugin module.
---
--- @param specs table[] each entry:
---   display    string : panel name (e.g. "ISS")
---   norad_id   integer: CelesTrak / NORAD catalog ID
---   color      string?: marker + label palette key (default "accent")
---   key        string?: single-char keybind to toggle this sat's visibility
---                       while the panel is focused. Optional; sats without
---                       a key stay always-visible.
function M.make(specs)
    -- Per-sat runtime state. Created once per Component instance —
    -- toggling the panel off and on rebuilds it (LuaComponent is
    -- re-created each push), which doubles as a "refresh TLE"
    -- shortcut without needing an extra command.
    local sats = {}
    for _, spec in ipairs(specs) do
        table.insert(sats, {
            display = spec.display,
            norad_id = spec.norad_id,
            color = spec.color or "accent",
            key = spec.key,
            visible = true, -- on by default; in-panel key toggles
            tle = nil,
            position = nil,
            fetch_job = nil,
        })
    end

    -- Map char → index for handle_event dispatch + footer hint list.
    local key_to_idx = {}
    local hints = {}
    for i, sat in ipairs(sats) do
        if sat.key then
            key_to_idx[sat.key] = i
            table.insert(hints, { sat.key, "toggle " .. sat.display })
        end
    end
    table.insert(hints, { "Enter", "centre on first visible" })

    -- Block::Borders::ALL eats one row top + one row bottom, so the
    -- visible content area is `height - 2`. Size for exactly N sat
    -- rows (no in-panel header — the block's `satellite` title bar
    -- already labels the panel). Width fits
    -- "○ [h] Hubble  XX.X°N, YYY.Y°E  ZZZkm".
    local panel_height = #sats + 2

    local initial_jump_done = false

    return {
        name = "satellite",
        label = "Toggle satellite",
        layout = { anchor = "top-left", width = 38, height = panel_height },
        footer_hints = hints,

        render = function()
            local lines = {}
            for _, sat in ipairs(sats) do
                local marker = sat.visible and "●" or "○"
                local body
                if not sat.visible then
                    body = "(off)"
                else
                    body = format_position(sat.position)
                end
                local key_hint = sat.key and ("[" .. sat.key .. "] ") or "    "
                table.insert(lines, string.format("%s %s%-7s %s",
                    marker, key_hint, sat.display, body))
            end
            return lines
        end,

        paint_on_map = function(map)
            for _, sat in ipairs(sats) do
                if sat.visible and sat.position then
                    map:point(sat.position.lon, sat.position.lat, "◉", sat.color)
                    map:label(sat.position.lon, sat.position.lat, " " .. sat.display, sat.color)
                end
            end
        end,

        handle_event = function(key)
            -- In-panel per-sat toggle. The bridge surfaces char keys
            -- as `code = "Char"` + `char = <c>` (see
            -- `key_code_to_lua` in `src/lua/component.rs`), so we
            -- match on `key.char`, not `key.code`. `return nil`
            -- consumes the event so it doesn't leak to the base
            -- layer (which uses `h` for pan-left, etc.).
            if key.code == "Char" and key.char and key_to_idx[key.char] then
                local idx = key_to_idx[key.char]
                sats[idx].visible = not sats[idx].visible
                return nil
            end
            if key.code == "Enter" then
                for _, sat in ipairs(sats) do
                    if sat.visible and sat.position then
                        ttymap.map:jump(sat.position.lon, sat.position.lat)
                        break
                    end
                end
                return nil
            end
            return { ignore = true }
        end,

        poll = function()
            for _, sat in ipairs(sats) do
                -- TLE fetch: kick off once, only for visible sats.
                -- An invisible sat that the user toggles on later
                -- starts its fetch the next poll tick — that's the
                -- right "lazy" semantics for a panel that may never
                -- need every entry.
                if sat.visible and not sat.tle and not sat.fetch_job then
                    sat.fetch_job = ttymap.http:fetch(tle_url(sat.norad_id))
                end
                if sat.fetch_job then
                    local body = sat.fetch_job:try_take()
                    if body then
                        sat.tle = ttymap.sgp4:parse_tle(body)
                        sat.fetch_job = nil
                    end
                end

                -- Re-propagate every poll while visible. SGP4 runs
                -- in microseconds; the wall-clock argument (nil →
                -- current) keeps motion smooth across frames.
                if sat.visible and sat.tle then
                    local pos = ttymap.sgp4:propagate(sat.tle)
                    if pos then sat.position = pos end
                end
            end

            -- Auto-recentre on the first sat that produces a position
            -- after the panel opens, so the marker is immediately
            -- visible without forcing the user to press Enter.
            if not initial_jump_done then
                for _, sat in ipairs(sats) do
                    if sat.visible and sat.position then
                        initial_jump_done = true
                        ttymap.map:jump(sat.position.lon, sat.position.lat)
                        break
                    end
                end
            end
        end,
    }
end

return M
