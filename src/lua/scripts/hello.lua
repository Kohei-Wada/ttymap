-- ttymap built-in Lua plugin demo.
--
-- A plugin module is just a table with at least `name` and `render`.
-- `render()` is called every frame; return a list of strings and the
-- host wraps them in a framed Paragraph titled with `name`.
--
-- `poll()` is optional. The host calls it every tick (regardless of
-- focus). Use it to advance counters, drain async fetches, etc.
-- Persistent host services hang off the `host` global:
--
--   host:fetch_url(url) -> Job   -- spawns a background HTTP GET
--   job:try_take()      -> string | nil  -- non-blocking
--   host:jump(lon, lat)            -- recentre the map (fire-and-forget)
--   host:parse_json(s)  -> value | nil   -- JSON → nested Lua tables
--
-- `handle_event(key)` is optional. The host calls it for every key
-- press while this component owns focus. The `key` table looks like:
--
--   { code  = "Char" | "Enter" | "Esc" | "Tab" | "Up" | "Down" | ...,
--     char  = "a",   -- only set when code == "Char"
--     ctrl  = bool, shift = bool, alt = bool }
--
-- Return value tells the host what to do:
--
--   nil                  -- silently consume the event (modal feel)
--   { close  = true }    -- pop this component off the stack
--   { ignore = true }    -- pass through to the base layer keymap
--
-- `paint_on_map(map)` is optional. The host calls it every frame to
-- give the plugin a chance to draw world-space markers. Use:
--
--   map:point(lon, lat, glyph, color)
--     -- glyph: single-cell character ("*", "@", "▲", ...)
--     -- color: "accent" | "accent_alt"   (theme-aware)
--
-- Future plugin authors: copy this file as a starting template.
-- Bridge surface today is text + keys + map markers; async fetch
-- and richer widgets land in follow-ups (see docs/lua-bridge-surface.md).

-- Throttled to one bump per wall-clock second: poll() runs at the
-- main loop's tick rate (~250 Hz today), and mutating the render
-- output every tick would force terminal redraws at that rate. Real
-- plugins update only on meaningful events (fetch arrival, user
-- input, etc.) — see `os.time()` resolution as a stand-in.
local state = { ticks = 0, last_second = 0 }

return {
    name = "hello",

    render = function()
        return {
            "Hello from Lua!",
            "",
            "This panel is rendered by",
            "src/lua/scripts/hello.lua",
            "",
            "ticks: " .. tostring(state.ticks),
            "",
            "Press Esc or q to close.",
        }
    end,

    poll = function()
        local now = os.time()
        if now ~= state.last_second then
            state.last_second = now
            state.ticks = state.ticks + 1
        end
    end,

    handle_event = function(key)
        if key.code == "Esc" then
            return { close = true }
        end
        if key.code == "Char" and key.char == "q" then
            return { close = true }
        end
        if key.code == "Enter" then
            -- Enter jumps to Tokyo so the map demo shows the
            -- request flowing back through the host.
            host:jump(139.7595, 35.6828)
        end
        -- Default: consume so the panel feels modal.
        return nil
    end,

    paint_on_map = function(map)
        -- Drop a couple of markers so the plugin is visible on the map
        -- as well as in the panel.
        map:point(139.7595,  35.6828, "*", "accent")     -- Tokyo
        map:point( 13.4050,  52.5200, "*", "accent")     -- Berlin
        map:point(-74.0060,  40.7128, "*", "accent_alt") -- New York
    end,
}
