-- notify — top-right always-on chrome that surfaces transient status
-- messages posted via `ttymap.notify(msg, opts)`.
--
-- The host owns the ring buffer (`ttymap.api.notify.recent(ttl_ms)`);
-- this plugin is the renderer. Newest entries appear at row 0;
-- older entries scroll down beneath, capped at MAX_VISIBLE so a
-- flood doesn't take over the screen. Display TTL is 3s.

local TTL_MS = 3000
local MAX_VISIBLE = 4

-- Map plugin-supplied levels to the three theme keywords MapApi
-- exposes. There's no dedicated red/orange in the current palette,
-- so error reuses `accent` (most visible) and warn picks the
-- alternate accent — info fades into `muted` so chatty plugins
-- don't dominate the corner.
local function color_for(level)
    if level == "error" then return "accent" end
    if level == "warn" then return "accent_alt" end
    return "muted"
end

ttymap.api.frame.on_tick(function(map)
    local entries = ttymap.api.notify.recent(TTL_MS)
    if #entries == 0 then return end

    -- `recent` returns oldest-first (push_back order); render newest
    -- at row 0 by iterating tail-to-head.
    local n = #entries
    local start = math.max(1, n - MAX_VISIBLE + 1)
    local row = 0
    for i = n, start, -1 do
        local e = entries[i]
        map:text_anchored("top-right", row, e.message, color_for(e.level))
        row = row + 1
    end
end)
