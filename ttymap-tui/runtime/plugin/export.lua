-- export — palette action that snapshots the current frame to disk.
ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function() ttymap.api.frame.export() end,
})
