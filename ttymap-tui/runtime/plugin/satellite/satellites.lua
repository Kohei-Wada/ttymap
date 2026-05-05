-- satellite.satellites — multi-sat tracker.
--
-- Each `make(specs)` call wires up one tracker:
--   * subscribes a `ttymap.api.frame.on_tick` callback that drives TLE
--     fetch + SGP4 propagation + map paint while the panel is open
--     (gated on the captured `w` card handle, same convention as
--     aircraft / wiki),
--   * returns an `open` / `close` / `toggle` trio the caller wires to a
--     palette command or keybind. The card owns per-sat visibility
--     keystrokes (`i` for ISS, `H` for Hubble, …) and C-n / C-p / Enter
--     focus navigation.
--
-- Two entry shapes (unchanged from the previous shape):
--
--   * **single** — one NORAD ID, one marker + label, one row showing
--     position + altitude. Use for ISS, Hubble, etc.
--   * **group** — a CelesTrak group endpoint (`stations`, `starlink`,
--     `science`, …). Returns N TLEs in one fetch; render as N markers
--     without labels (would clutter for thousands of sats) and a row
--     showing the live count. Uses `ttymap.sgp4:propagate_batch` so
--     N propagations cross the Lua/Rust boundary in one call.
--
-- One palette entry, one card, regardless of how many entries the
-- consumer configures. Visibility flags survive a panel toggle —
-- `make` runs once at plugin registration, so per-sat `visible` state
-- persists across open/close cycles. Toggling the panel does NOT
-- re-trigger TLE fetches; the disk-cached `fetch_cached` reuses the
-- prior body for an hour anyway, so this is a non-issue.

local sidebar = require "ttymap.sidebar"

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

--- First lon/lat that actually represents a position for `sat`, or
--- nil if there's nothing to jump to yet (no propagation, group
--- still fetching, all-`false` batch result, …).
local function focus_point(sat)
    if not sat or not sat.visible then return nil end
    if sat.kind == "single" then
        if sat.positions then
            return sat.positions.lon, sat.positions.lat
        end
    else
        if sat.positions then
            for _, p in ipairs(sat.positions) do
                if p then return p.lon, p.lat end
            end
        end
    end
    return nil
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
---
--- @return table { open, close, toggle } — caller wires `toggle`
---   (or `open` / `close` individually) to a palette command /
---   keybind. The factory itself subscribes the per-frame callback
---   via `ttymap.api.frame.on_tick`.
function M.make(specs)
    -- Per-entry runtime state. Lives for the program lifetime;
    -- visibility flags persist across panel toggles. TLE fetches are
    -- one-shot per sat (re-fetched only if `tles` is cleared, which
    -- nothing currently does — `fetch_cached` handles staleness).
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

    -- Map char → index for handle_key dispatch + window-local
    -- footer hint list (visible only while the panel is focused).
    local key_to_idx = {}
    local hints = {}
    for i, sat in ipairs(sats) do
        if sat.key then
            key_to_idx[sat.key] = i
            table.insert(hints, { sat.key, "toggle " .. sat.display })
        end
    end
    table.insert(hints, { "C-n/C-p", "focus" })
    table.insert(hints, { "Enter", "re-centre" })
    table.insert(hints, { "Esc/q", "close" })

    local selected = 1
    local initial_jump_done = false

    -- Helpers shared by render / paint / loop. Inlined as locals so
    -- they capture the same `sats` upvalue.

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

    -- Move focus to the next visible sat in `direction` (+1 / -1).
    -- Wraps. Skips invisible entries; no-op when nothing's visible.
    -- On a successful move, jumps the map to that sat's position
    -- (if known yet) — auto-jump is the whole point of the binding.
    local function move_selection(direction)
        local n = #sats
        if n == 0 then return end
        local idx = selected
        for _ = 1, n do
            idx = idx + direction
            if idx < 1 then
                idx = n
            elseif idx > n then
                idx = 1
            end
            if sats[idx].visible then
                selected = idx
                local lon, lat = focus_point(sats[idx])
                if lon then ttymap.map:jump(lon, lat) end
                return
            end
        end
        -- All invisible: leave `selected` as-is.
    end

    -- One sat = one 1-line item. The bridge highlights the selected
    -- row natively; this builder just describes the row in default
    -- styling.
    local function build_items()
        local items = {}
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
            local row = string.format("%s %s%-8s %s",
                marker, key_hint, sat.display, body)
            table.insert(items, { row })
        end
        return items
    end

    local w = nil  -- card handle while open; nil while closed (also
                   -- the enabled flag for the loop)

    local function paint_markers(map)
        -- Spatial dedup for group entries: at low zoom Starlink-class
        -- constellations pack hundreds of sats into a few pixels and
        -- the map turns into solid colour. Bucket positions into a
        -- degree-grid that scales with zoom so only one sat per visible
        -- cell renders. 5° at zoom 0 (~world view) → 0.005° at zoom 10.
        -- Same icon (`◉`) as single sats — by the time dedup runs,
        -- count is low enough that a real marker reads fine.
        local zoom = map:zoom()
        local cell_deg = 5 / (2 ^ zoom)

        for i, sat in ipairs(sats) do
            if not sat.visible then goto continue end
            local color = (i == selected) and "accent_alt" or sat.color
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
                                map:point(p.lon, p.lat, "◉", color)
                            end
                        end
                    end
                end
            else
                if sat.positions then
                    map:point(sat.positions.lon, sat.positions.lat, "◉", color)
                    map:label(sat.positions.lon, sat.positions.lat, " " .. sat.display, color)
                end
            end
            ::continue::
        end
    end

    local function step(_map)
        for _, sat in ipairs(sats) do
            -- TLE fetch: kick off once, only for visible sats.
            -- An invisible entry that the user toggles on later
            -- starts its fetch the next loop tick.
            if sat.visible and not sat.tles and not sat.fetch_job then
                local url = sat.kind == "group"
                    and group_url(sat.group)
                    or single_url(sat.norad_id)
                -- Disk-cached fetch with a 1h freshness window.
                -- CelesTrak's gp.php refreshes every ~2h and 403s
                -- a same-IP repeat fetch within that window — so
                -- a normal `fetch` would strand us on "awaiting"
                -- on every restart inside a single CelesTrak
                -- bucket. `fetch_cached` reads the prior body
                -- from disk first, falls back to it on HTTP
                -- error, and only round-trips when stale.
                sat.fetch_job = ttymap.http:fetch_cached(url, 3600)
            end
            if sat.fetch_job then
                local body = sat.fetch_job:try_take()
                if body then
                    if sat.kind == "group" then
                        sat.tles = ttymap.sgp4:parse_tles(body)
                    else
                        sat.tles = ttymap.sgp4:parse_tle(body)
                    end
                    if not sat.tles then
                        ttymap.notify(string.format(
                            "satellite: TLE parse failed for %s",
                            sat.display or "satellite"
                        ), { level = "warn" })
                    elseif sat.kind == "group" then
                        ttymap.notify(string.format(
                            "satellite: loaded %d TLEs (%s)",
                            #sat.tles, sat.display or sat.group or "?"
                        ))
                    else
                        ttymap.notify(string.format(
                            "satellite: loaded %s",
                            sat.display or "satellite"
                        ))
                    end
                    sat.fetch_job = nil
                end
            end

            -- Re-propagate every loop tick while visible. Pure-Rust
            -- SGP4 runs in microseconds; passing nil for the time
            -- arg uses sub-second wall-clock for smooth motion.
            -- `propagate_batch` keeps a few-thousand-sat group to
            -- one Lua/Rust crossing per frame.
            if sat.visible and sat.tles then
                if sat.kind == "group" then
                    sat.positions = ttymap.sgp4:propagate_batch(sat.tles)
                else
                    local pos = ttymap.sgp4:propagate(sat.tles)
                    if pos then sat.positions = pos end
                end
            end
        end

        -- Auto-recentre on the first usable position after the panel
        -- opens, so the marker is immediately visible without forcing
        -- the user to navigate. `initial_jump_done` is reset on close
        -- so each (re)open re-centres.
        if not initial_jump_done then
            for _, sat in ipairs(sats) do
                local lon, lat = focus_point(sat)
                if lon then
                    initial_jump_done = true
                    ttymap.map:jump(lon, lat)
                    break
                end
            end
        end
    end

    local function close()
        if w then
            w:close()
            w = nil
            initial_jump_done = false
        end
    end

    local function open()
        if w then return end
        local function selected_index()
            return selected
        end

        w = ttymap.api.card.open({
            name = "satellite",
            footer_hints = hints,
            items    = build_items,
            selected = selected_index,
            handle_key = function(key)
                local code = key.code
                local ch = key.char
                local ctrl = key.ctrl

                -- In-panel per-entry visibility toggle. Char keys
                -- come through as `code = "Char"` + `char = <c>`;
                -- match on `key.char`. `return nil` consumes so the
                -- event doesn't leak to the base layer (which uses
                -- `h` for pan-left etc.). Skip when ctrl is held so
                -- C-n / C-p reach the focus-navigation branch even
                -- if a sat is bound to `n` or `p`. Runs before
                -- `sidebar.is_close_key` so a sat bound to `q` still
                -- toggles its visibility instead of closing.
                if code == "Char" and ch and not ctrl and key_to_idx[ch] then
                    local idx = key_to_idx[ch]
                    sats[idx].visible = not sats[idx].visible
                    return nil
                end

                if sidebar.up_pressed(key) then
                    move_selection(-1)
                    return nil
                end
                if sidebar.down_pressed(key) then
                    move_selection(1)
                    return nil
                end

                if code == "Enter" then
                    -- Re-centre on the focused sat — handy after the
                    -- user pans away to look at something else.
                    local lon, lat = focus_point(sats[selected])
                    if lon then ttymap.map:jump(lon, lat) end
                    return nil
                end

                if sidebar.is_close_key(key) then
                    close()
                    return nil
                end

                return { ignore = true }
            end,
        })
    end

    local function toggle()
        if w then close() else open() end
    end

    -- Per-frame work runs only while the panel is open: drains
    -- TLE fetches, propagates SGP4 for visible sats, paints markers.
    -- Closing the panel (`w = nil`) immediately stops propagation
    -- and the markers vanish, mirroring the legacy "Component
    -- pushed only while open" behavior.
    ttymap.api.frame.on_tick(function(map)
        if not w then return end
        step(map)
        paint_markers(map)
    end)

    return {
        open = open,
        close = close,
        toggle = toggle,
    }
end

return M
