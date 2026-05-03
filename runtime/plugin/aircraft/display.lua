-- aircraft.display — formatting helpers for the aircraft panel +
-- markers. Pure functions; no state, no I/O.

local M = {}

-- Map a true-track in degrees (0 = north, clockwise) to one of
-- eight unicode arrows. Each arrow covers a 45° sector centred on
-- its cardinal/intercardinal direction; e.g. north spans
-- [337.5°, 22.5°). Mirrors the helper that lived in the Rust
-- plugin before the takeover.
local ARROWS = { "↑", "↗", "→", "↘", "↓", "↙", "←", "↖" }

local function heading_arrow(deg)
    local n = deg % 360
    if n < 0 then n = n + 360 end
    local sector = math.floor((n + 22.5) / 45) % 8
    return ARROWS[sector + 1]
end

-- Structured line for the panel — a vec of `{ text, style }` spans
-- matching the shape `LuaCardComponent` understands. The bridge
-- highlights the selected row natively (List + ListState), so this
-- helper just paints the title in `accent` and leaves selection to
-- the host. Secondary info (altitude, ground state) renders muted.
function M.fmt(a)
    local cs = a.callsign ~= "" and a.callsign or "(no callsign)"
    local secondary = ""
    if a.on_ground then
        secondary = "  (ground)"
    elseif type(a.alt) == "number" then
        secondary = string.format("  %dm", math.floor(a.alt))
    end
    return {
        { text = cs,        style = "accent" },
        { text = secondary, style = "muted" },
    }
end

function M.marker_for(a)
    if a.on_ground then return "◇" end
    if type(a.heading) == "number" then return heading_arrow(a.heading) end
    return "◆"
end

return M
