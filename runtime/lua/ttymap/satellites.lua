-- ttymap.satellites — multi-sat tracker as a single Component.
--
-- Each Component instance aggregates N entries (configured by the
-- consumer), shares one panel for status display, and toggles per-
-- entry visibility via in-panel keystrokes. Two entry shapes:
--
--   * **single** — one NORAD ID, one marker + label, one row showing
--     position + altitude. Use for ISS, Hubble, etc.
--   * **group** — a CelesTrak group endpoint (`stations`, `starlink`,
--     `science`, …). Returns N TLEs in one fetch; render as N markers
--     without labels (would clutter for thousands of sats) and a row
--     showing the live count. Uses `ttymap.sgp4:propagate_batch` so
--     N propagations cross the Lua/Rust boundary in one call.
--
-- One palette entry, one window, regardless of how many entries the
-- consumer configures.

local M = {}

local function single_url(norad_id)
    return string.format(
        "https://celestrak.org/NORAD/elements/gp.php?CATNR=%d&FORMAT=tle",
        norad_id)
end

local function group_url(group)
    return string.format(
        "https://celestrak.org/NORAD/elements/gp.php?GROUP=%s&FORMAT=tle",
        group)
end

local function format_position(pos)
    if not pos then return "(awaiting…)" end
    return string.format("%.1f°N, %.1f°E  %dkm",
        pos.lat, pos.lon,
        math.floor(pos.alt_km + 0.5))
end

--- Build a multi-satellite tracker plugin module.
---
--- @param specs table[] each entry has either:
---   norad_id   integer: single sat by NORAD catalog ID, or
---   group      string : CelesTrak group name (e.g. "starlink")
---   display    string : panel name + map label (e.g. "ISS", "Starlink")
---   color      string?: marker palette key (default "accent")
---   key        string?: single-char keybind to toggle visibility
---                       while the panel is focused. Optional; sats
---                       without a key stay always-visible.
function M.make(specs)
    -- Per-entry runtime state. Created once per Component instance —
    -- toggling the panel off and on rebuilds it (LuaComponent is
    -- re-created each push), which doubles as a "refresh TLE"
    -- shortcut without needing an extra command.
    local sats = {}
    for _, spec in ipairs(specs) do
        table.insert(sats, {
            display = spec.display,
            kind = spec.group and "group" or "single",
            norad_id = spec.norad_id,
            group = spec.group,
            color = spec.color or "accent",
            key = spec.key,
            visible = true,    -- on by default; in-panel key toggles
            tles = nil,        -- single → single handle, group → array
            positions = nil,   -- single → one {lon,lat,...}, group → array
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
    -- visible content area is `height - 2`. Size for exactly N rows
    -- (no in-panel header — the block's `satellite` title bar
    -- already labels the panel). Width fits both
    -- "○ [h] Hubble  XX.X°N, YYY.Y°E  ZZZkm" and
    -- "○ [s] Starlink  6234 sats".
    local panel_height = #sats + 2

    local initial_jump_done = false

    -- Helpers shared by render / paint_on_map / poll. Inlined as
    -- local functions so they capture the same `sats` upvalue.

    local function single_body(sat)
        return format_position(sat.positions)
    end

    local function group_body(sat)
        if not sat.tles then return "(awaiting…)" end
        if not sat.positions then return "(propagating…)" end
        local count = 0
        for _, p in ipairs(sat.positions) do
            if p then count = count + 1 end
        end
        return string.format("%d sats", count)
    end

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
                elseif sat.kind == "group" then
                    body = group_body(sat)
                else
                    body = single_body(sat)
                end
                local key_hint = sat.key and ("[" .. sat.key .. "] ") or "    "
                table.insert(lines, string.format("%s %s%-8s %s",
                    marker, key_hint, sat.display, body))
            end
            return lines
        end,

        paint_on_map = function(map)
            -- Spatial dedup for group entries: at low zoom Starlink-
            -- class constellations pack hundreds of sats into a few
            -- pixels and the map turns into solid colour. Bucket
            -- positions into a degree-grid that scales with zoom so
            -- only one sat per visible cell renders. 5° at zoom 0
            -- (~world view) → 0.005° at zoom 10. Same icon (`◉`) as
            -- single sats — by the time dedup runs, count is low
            -- enough that a real marker reads fine.
            local zoom = map:zoom()
            local cell_deg = 5 / (2 ^ zoom)

            for _, sat in ipairs(sats) do
                if not sat.visible then goto continue end
                if sat.kind == "group" then
                    if sat.positions then
                        local seen = {}
                        for _, p in ipairs(sat.positions) do
                            if p then
                                local kx = math.floor(p.lon / cell_deg)
                                local ky = math.floor(p.lat / cell_deg)
                                local k = kx .. ":" .. ky
                                if not seen[k] then
                                    seen[k] = true
                                    map:point(p.lon, p.lat, "◉", sat.color)
                                end
                            end
                        end
                    end
                else
                    if sat.positions then
                        map:point(sat.positions.lon, sat.positions.lat, "◉", sat.color)
                        map:label(sat.positions.lon, sat.positions.lat, " " .. sat.display, sat.color)
                    end
                end
                ::continue::
            end
        end,

        handle_event = function(key)
            -- In-panel per-entry toggle. Char keys come through the
            -- bridge as `code = "Char"` + `char = <c>` — match on
            -- `key.char`, not `key.code`. `return nil` consumes the
            -- event so it doesn't leak to the base layer (which uses
            -- `h` for pan-left, etc.).
            if key.code == "Char" and key.char and key_to_idx[key.char] then
                local idx = key_to_idx[key.char]
                sats[idx].visible = not sats[idx].visible
                return nil
            end
            if key.code == "Enter" then
                for _, sat in ipairs(sats) do
                    if sat.visible then
                        if sat.kind == "single" and sat.positions then
                            ttymap.map:jump(sat.positions.lon, sat.positions.lat)
                            break
                        elseif sat.kind == "group" and sat.positions then
                            for _, p in ipairs(sat.positions) do
                                if p then
                                    ttymap.map:jump(p.lon, p.lat)
                                    break
                                end
                            end
                            break
                        end
                    end
                end
                return nil
            end
            return { ignore = true }
        end,

        poll = function()
            for _, sat in ipairs(sats) do
                -- TLE fetch: kick off once, only for visible sats.
                -- An invisible entry that the user toggles on later
                -- starts its fetch the next poll tick.
                if sat.visible and not sat.tles and not sat.fetch_job then
                    local url = sat.kind == "group"
                        and group_url(sat.group)
                        or single_url(sat.norad_id)
                    sat.fetch_job = ttymap.http:fetch(url)
                end
                if sat.fetch_job then
                    local body = sat.fetch_job:try_take()
                    if body then
                        if sat.kind == "group" then
                            sat.tles = ttymap.sgp4:parse_tles(body)
                        else
                            sat.tles = ttymap.sgp4:parse_tle(body)
                        end
                        sat.fetch_job = nil
                    end
                end

                -- Re-propagate every poll while visible. Pure-Rust
                -- SGP4 runs in microseconds; passing nil for the
                -- time arg uses sub-second wall-clock for smooth
                -- motion. `propagate_batch` keeps a few-thousand-sat
                -- group to one Lua/Rust crossing per frame.
                if sat.visible and sat.tles then
                    if sat.kind == "group" then
                        sat.positions = ttymap.sgp4:propagate_batch(sat.tles)
                    else
                        local pos = ttymap.sgp4:propagate(sat.tles)
                        if pos then sat.positions = pos end
                    end
                end
            end

            -- Auto-recentre on the first usable position after the
            -- panel opens, so the marker is immediately visible
            -- without forcing the user to press Enter.
            if not initial_jump_done then
                for _, sat in ipairs(sats) do
                    if not sat.visible then goto skip end
                    if sat.kind == "single" and sat.positions then
                        initial_jump_done = true
                        ttymap.map:jump(sat.positions.lon, sat.positions.lat)
                        break
                    elseif sat.kind == "group" and sat.positions then
                        for _, p in ipairs(sat.positions) do
                            if p then
                                initial_jump_done = true
                                ttymap.map:jump(p.lon, p.lat)
                                break
                            end
                        end
                        if initial_jump_done then break end
                    end
                    ::skip::
                end
            end
        end,
    }
end

return M
