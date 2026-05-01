-- export (Lua port) — palette action that snapshots the current
-- frame to disk as an ANSI-escape text file.
--
-- Headless: pushed onto the stack on palette select, fires
-- `ttymap.window:export_frame()` once on its first poll, then self-closes.
-- The actual file write lives in `App::dispatch` (it owns both the
-- latest `MapFrame` and the cache directory); this script only
-- triggers the AppMsg.

local state = { fired = false }

ttymap.register_plugin({
    name = "export",
    handle_event = function(_)
        return { ignore = true }
    end,

    poll = function()
        if state.fired then return end
        state.fired = true
        ttymap.window:export_frame()
        ttymap.window:close()
    end,
})

ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function() ttymap.plugin:open() end,
})
