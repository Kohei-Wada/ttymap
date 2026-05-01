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

function M.fmt(a, selected)
    local prefix = selected and "→ " or "  "
    local cs     = a.callsign ~= "" and a.callsign or "(no callsign)"
    local alt    = ""
    if type(a.alt) == "number" then
        alt = string.format(" %dm", math.floor(a.alt))
    end
    local ground = a.on_ground and " (ground)" or ""
    return prefix .. cs .. alt .. ground
end

function M.marker_for(a)
    if a.on_ground then return "◇" end
    if type(a.heading) == "number" then return heading_arrow(a.heading) end
    return "◆"
end

return M
