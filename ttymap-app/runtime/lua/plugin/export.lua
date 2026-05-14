-- export — palette action that snapshots the current frame to disk.
--
-- The host hands over the rendered ANSI bytes via
-- `ttymap.api.frame.to_ansi()`; everything below — path choice,
-- success / failure notification — is plugin-side policy. Nothing
-- file-related lives in Rust; this script is the canonical example
-- of the "Rust hands the bytes, Lua decides what to do with them"
-- split.
--
-- Default destination is `/tmp` because ttymap exports are
-- throwaway snapshots, not long-term saves; users who want them
-- under `$XDG_DATA_HOME` can override this plugin in their own
-- runtime path (a same-named `plugin/export.lua` shadows this one).

local function build_path()
    return string.format("/tmp/ttymap-%s.ans", os.date("%Y%m%d-%H%M%S"))
end

ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function()
        local ansi = ttymap.api.frame.to_ansi()
        if not ansi then
            ttymap.notify("export: no frame to write yet", { level = "warn" })
            return
        end

        local path = build_path()
        local f, err = io.open(path, "w")
        if not f then
            ttymap.notify("export: " .. (err or "open failed"), { level = "error" })
            return
        end
        f:write(ansi)
        f:close()
        ttymap.notify("Frame exported to " .. path, { level = "info" })
    end,
})
