-- help — keybinding cheatsheet shown as a centred popup.
--
-- Modal: any key closes it. The cheatsheet is built lazily at
-- render time from `ttymap.help:keymap_entries()` (live keymap
-- actions) and `ttymap.help:palette_entries()` (sibling plugins'
-- palette hints), so registration order doesn't matter.
--
-- Pure-action plugin: no `on_tick` (static cheatsheet, no async
-- work, no map paint). Just a palette command + keybind that open
-- the popup window via `ttymap.api.card.open`.

local sidebar = require "ttymap.sidebar"

local URL_MAPSCII = "https://github.com/rastapasta/mapscii"
local URL_HOME = "https://github.com/Kohei-Wada/ttymap"

local w = nil  -- popup handle while open; nil while closed

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

    for _, entry in ipairs(ttymap.help:keymap_entries()) do
        table.insert(lines, key_line(entry.key, entry.label))
    end

    table.insert(lines, blank_line())
    table.insert(lines, key_line("gg", "Zoom to world"))
    table.insert(lines, key_line("Tab/S-Tab", "Cycle focus"))
    table.insert(lines, key_line(":", "Command palette"))
    for _, entry in ipairs(ttymap.help:palette_entries()) do
        table.insert(lines, key_line(entry.key, entry.label))
    end
    table.insert(lines, blank_line())
    table.insert(lines, key_line("Drag / Scroll", "Pan / zoom (mouse)"))
    table.insert(lines, blank_line())
    table.insert(lines, text_line(" Bug reports and pull requests welcome:"))
    table.insert(lines, url_line(URL_HOME))

    return lines
end

local function close()
    if w then
        w:close()
        w = nil
    end
end

local function open()
    if w then return end
    w = ttymap.api.card.open({
        name = "help",
        footer_hints = {
            { key = "↑↓ PgUp PgDn", label = "scroll" },
            { key = "q / Esc",      label = "close" },
        },
        render = build_lines,
        handle_key = function(key)
            if sidebar.is_close_key(key) then
                close()
                return nil
            end
            -- Everything else (j/k pan, : palette …) falls through
            -- to the base layer. The bridge intercepts ↑ ↓ PgUp
            -- PgDn Home End C-n C-p for built-in section scroll
            -- because help has no selection logic of its own.
            return { ignore = true }
        end,
    })
end

local function toggle()
    if w then close() else open() end
end

ttymap.register_palette_command({ label = "Toggle help", invoke = toggle })
ttymap.register_keybind("?", toggle)
