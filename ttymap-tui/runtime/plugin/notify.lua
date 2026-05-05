-- notify — popup-style renderer for the host's "notify" bus event.
--
-- Producers (Lua `ttymap.notify(msg, opts)` or any Rust caller via
-- `LuaHandle::notify`) publish `Event::Notify { message, level }` on
-- the bus. We subscribe via `ttymap.on_event("notify", ...)`, buffer
-- entries with their post time, and paint top-left as one bordered
-- popup per message for `TTL_S` after arrival. Newest at the top;
-- a blank row between popups separates them visually.
--
-- All popups render in `accent` regardless of `level` — severity
-- gets conveyed by the message text (plugin authors free to prefix
-- "warn:" / "error:" / icons themselves). Theme-coloured severity
-- mapping was tried and dropped: `accent_alt` is cyan in DARK and
-- red in BRIGHT, which inverts the warn/error semantic across
-- themes. `level` stays in the event payload so future subscribers
-- (audit log, sound, etc) can still filter by it.
--
-- The ring lives entirely here — Rust side has no notify state at
-- all, just the generic event bus.

-- TTL in **seconds** because `os.time()` is our clock; `os.clock()`
-- (process CPU time, not wall) doesn't advance while the TUI idles
-- between events, which left popups stuck on screen forever. 1 s
-- resolution is fine — at TTL=4 the popup disappears in 3–4 s.
local TTL_S = 4
local RING_CAP = 32
local MAX_TEXT_WIDTH = 60  -- truncate long messages so the popup
                           -- doesn't dominate the map area

-- Wall-clock seconds. `os.time()` ticks even while the host idles,
-- which is what we need for "expire after N seconds even if the
-- user does nothing" — process CPU time (`os.clock`) does not.
local function now_s()
    return os.time()
end

-- Display-width count. `utf8.len` returns code-point count, which
-- equals display columns for typical ASCII / Latin / box-drawing
-- chars. CJK wide chars (display width 2) would still under-count
-- by 1 cell each — fine for our common case (ASCII path / English
-- messages). Falls back to byte length on malformed UTF-8.
local function dwidth(s)
    return utf8.len(s) or #s
end

-- Truncate by display width. ASCII `...` (3 cells) keeps the
-- border math honest — using `…` (1 cell, 3 bytes) was the source
-- of the misalignment bug since `#str` ≠ display width.
local function clip(s)
    if dwidth(s) <= MAX_TEXT_WIDTH then return s end
    -- byte-based sub is safe here because we keep MAX_TEXT_WIDTH-3
    -- ASCII bytes from the head; non-ASCII upstream callers rarely
    -- hit this path.
    return s:sub(1, MAX_TEXT_WIDTH - 3) .. "..."
end

local entries = {}

ttymap.on_event("notify", function(e)
    table.insert(entries, {
        message = clip(e.message),
        level = e.level,
        posted_at = now_s(),
    })
    if #entries > RING_CAP then
        table.remove(entries, 1)
    end
end)

ttymap.api.frame.on_tick(function(map)
    if #entries == 0 then return end

    -- Prune expired entries from the head.
    local now = now_s()
    while #entries > 0 and now - entries[1].posted_at >= TTL_S do
        table.remove(entries, 1)
    end
    if #entries == 0 then return end

    -- One popup per entry, newest at the top. 1-row gap between
    -- popups so borders don't visually run together.
    local row = 0
    local n = #entries
    for i = n, 1, -1 do
        local e = entries[i]
        local inner = " " .. e.message .. " "
        local border = string.rep("─", dwidth(inner))
        map:text_anchored("top-left", row,     "╭" .. border .. "╮", "accent")
        map:text_anchored("top-left", row + 1, "│" .. inner .. "│",  "accent")
        map:text_anchored("top-left", row + 2, "╰" .. border .. "╯", "accent")
        row = row + 3
        if i > 1 then row = row + 1 end  -- gap before next popup
    end
end)
