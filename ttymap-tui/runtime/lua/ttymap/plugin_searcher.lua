-- ttymap.plugin_searcher — Lua-side resolver for `<layer>/plugin/...`
-- requires.
--
-- The host (Rust) does NOT know about the `plugin/` subdirectory
-- convention. Rust exposes one primitive — `ttymap.runtime_path`
-- (the resolved layer list) — and this module turns it into a
-- `package.searchers` entry.
--
-- "Plugin" is purely a Lua-side organisational unit: a `.lua` file
-- that calls some combination of `ttymap.register_palette_command`
-- / `ttymap.register_keybind` / `ttymap.on_event` to add itself to
-- the host's runtime API. There is no host-side plugin identity —
-- the searcher simply loads the file as a plain chunk and returns
-- whatever it returned (or `true` for the top-level case so Lua
-- caches the require).
--
-- A user on a layer that doesn't follow the `plugin/` convention
-- can drop a replacement at
-- `~/.config/ttymap/lua/ttymap/plugin_searcher.lua` and the
-- runtime-path searcher will pick it up first (user > bundled).

local M = {}

local function open_first(rel, layers)
    for _, layer in ipairs(layers) do
        for _, cand in ipairs({
            layer .. "/plugin/" .. rel .. ".lua",
            layer .. "/plugin/" .. rel .. "/init.lua",
        }) do
            local f = io.open(cand, "r")
            if f then
                local source = f:read("*a")
                f:close()
                return source, cand
            end
        end
    end
    return nil
end

function M.install()
    local layers = ttymap.runtime_path or {}
    table.insert(package.searchers, 2, function(name)
        local rel = name:gsub("%.", "/")
        local source, path = open_first(rel, layers)
        if not source then
            return "\n\tno plugin '" .. name .. "' in runtime path"
        end
        return function()
            local chunk = assert(load(source, path))
            return chunk()
        end
    end)
end

return M
