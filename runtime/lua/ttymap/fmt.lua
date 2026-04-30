-- ttymap.fmt — formatting helpers shared across bundled plugins.
--
-- Loaded via the `package.searchers` entry registered in
-- `src/lua/mod.rs::install_builtin_searcher`. Bundled plugins do
-- `local fmt = require "ttymap.fmt"`; user plugins can do the same
-- because the searcher consults the binary's BUILTIN_LIB_SCRIPTS map
-- before falling through to filesystem `package.path`.

local M = {}

--- Format a distance in metres as a short human-readable string.
--- < 1 km → integer metres ("823m"); ≥ 1 km → one-decimal kilometres
--- ("1.2km").
function M.distance(meters)
    if meters < 1000 then
        return string.format("%dm", math.floor(meters + 0.5))
    end
    return string.format("%.1fkm", meters / 1000)
end

return M
