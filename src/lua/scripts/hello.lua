-- ttymap built-in Lua plugin demo.
--
-- A plugin module is just a table with at least `name` and `render`.
-- `render()` is called every frame; return a list of strings and the
-- host wraps them in a framed Paragraph titled with `name`.
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
-- Future plugin authors: copy this file as a starting template.
-- Bridge surface today is text + keys; map paint, async fetch, and
-- richer widgets land in follow-ups (see docs/lua-bridge-surface.md).

return {
    name = "hello",

    render = function()
        return {
            "Hello from Lua!",
            "",
            "This panel is rendered by",
            "src/lua/scripts/hello.lua",
            "",
            "Press Esc or q to close.",
        }
    end,

    handle_event = function(key)
        if key.code == "Esc" then
            return { close = true }
        end
        if key.code == "Char" and key.char == "q" then
            return { close = true }
        end
        -- Default: consume so the panel feels modal.
        return nil
    end,
}
