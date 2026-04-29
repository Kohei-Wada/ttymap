-- help — keybinding cheatsheet shown as a centred popup.
--
-- Modal: any key closes it. The cheatsheet is built lazily at
-- render time from `host:keymap_entries()` (live keymap actions)
-- and `host:plugin_palette_entries()` (sibling plugins' palette
-- hints), so registration order doesn't matter.

local URL_MAPSCII = "https://github.com/rastapasta/mapscii"
local URL_HOME = "https://github.com/Kohei-Wada/ttymap"

-- Pad `s` on the right with spaces so column-aligned cheatsheet
-- rows line up under each other.
local function lpad_right(s, width)
    if #s >= width then return s end
    return s .. string.rep(" ", width - #s)
end

local function text_line(t)
    return { { text = t, style = "body" } }
end

local function url_line(u)
    return {
        { text = " ",  style = "body" },
        { text = u,    style = "link" },
    }
end

local function blank_line()
    return { { text = "", style = "body" } }
end

local function key_line(key, label)
    return {
        { text = " ",                    style = "body" },
        { text = lpad_right(key, 20),    style = "accent" },
        { text = " " .. label,           style = "body" },
    }
end

local function build_lines()
    local lines = {
        text_line(" A terminal-based map viewer — Mapbox vector tiles"),
        text_line(" rendered as Unicode Braille."),
        text_line(" Inspired by and built on ideas from mapscii:"),
        url_line(URL_MAPSCII),
        blank_line(),
    }

    for _, entry in ipairs(host:keymap_entries()) do
        table.insert(lines, key_line(entry.key, entry.label))
    end

    table.insert(lines, blank_line())
    table.insert(lines, key_line("gg", "Zoom to world"))
    table.insert(lines, key_line("Tab/S-Tab", "Cycle focus"))
    table.insert(lines, key_line(":", "Command palette"))
    for _, entry in ipairs(host:plugin_palette_entries()) do
        table.insert(lines, key_line(entry.key, entry.label))
    end
    table.insert(lines, blank_line())
    table.insert(lines, key_line("Drag / Scroll", "Pan / zoom (mouse)"))
    table.insert(lines, blank_line())
    table.insert(lines, text_line(" Bug reports and pull requests welcome:"))
    table.insert(lines, url_line(URL_HOME))

    return lines
end

return {
    name = "help",
    label = "Toggle help",
    key = "?",
    layout = { anchor = "center", width = 64, height = 22 },

    footer_hints = {
        { "any key", "close" },
    },

    render = function()
        return build_lines()
    end,

    handle_event = function(_)
        return { close = true }
    end,
}
