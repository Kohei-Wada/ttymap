-- ttymap.user_config — Lua-side user init.lua loader.
--
-- The bundled `runtime/init.lua` calls `.load()` partway through
-- so the user's `~/.config/ttymap/init.lua` runs in the shared VM
-- with full API access (`ttymap.opt`, `ttymap.keymap`, `require` of
-- bundled / user plugins, `package.loaded.X = true` to skip a
-- bundled plugin, etc.).
--
-- The host (Rust) does NOT resolve the user config path; this
-- module owns that responsibility, so the entry-point is
-- end-to-end Lua. XDG-style resolution: `$XDG_CONFIG_HOME/ttymap/init.lua`
-- if that env var is set, else `$HOME/.config/ttymap/init.lua`.
-- A user on macOS who wants `~/Library/Application Support` instead
-- can replace this lib (drop a copy at `~/.config/ttymap/lua/ttymap/user_config.lua`,
-- which takes priority via runtime-path layering).
--
-- Missing file = silent no-op. Lua errors during dofile are
-- pcall-recovered and forwarded to the host log so a broken user
-- init.lua doesn't crash the host.

local M = {}

local function user_init_path()
    local xdg = os.getenv("XDG_CONFIG_HOME")
    if xdg and xdg ~= "" then
        return xdg .. "/ttymap/init.lua"
    end
    local home = os.getenv("HOME")
    if home and home ~= "" then
        return home .. "/.config/ttymap/init.lua"
    end
    return nil
end

function M.load()
    local path = user_init_path()
    if not path then return end
    local f = io.open(path, "r")
    if not f then
        return  -- no user config, that's fine
    end
    f:close()
    local ok, err = pcall(dofile, path)
    if not ok then
        ttymap.log:warn("init.lua: " .. path .. " failed: " .. tostring(err))
    end
end

return M
